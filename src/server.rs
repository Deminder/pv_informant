use std::convert::Infallible;
use std::str::FromStr;

use crate::api_baderr;
use crate::context::Context;
use crate::errors::{ApiError, GenericError, Result};
use crate::excess_handler::ExcessRequestHandler;
use crate::interval_handler::IntervalRequestHandler;
use crate::report_handler::ReportRequestHandler;
use hyper::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_LENGTH};
use hyper::service::{make_service_fn, service_fn};
use hyper::{
    body::to_bytes, header, server::conn::AddrStream, Body, Method, Request, Response, Server,
    StatusCode,
};
use log::{error, info, warn};
use serde::de::DeserializeOwned;
use serde::Serialize;

pub struct InformantServer {
    context: Context,
}
impl InformantServer {
    pub fn new(context: Context) -> Self {
        InformantServer { context }
    }
}

#[async_trait]
pub trait HyperServerWrapper {
    async fn serve(&self) -> std::result::Result<(), hyper::Error>;
}

#[async_trait]
impl HyperServerWrapper for InformantServer {
    async fn serve(&self) -> std::result::Result<(), hyper::Error> {
        let context = &self.context.clone();
        let service = make_service_fn(move |stream: &AddrStream| {
            let mut context = context.clone();
            context.remote_addr = Some(stream.remote_addr());

            async {
                Ok::<_, Infallible>(service_fn(move |req| {
                    route_request(req, context.to_owned())
                }))
            }
        });
        Server::bind(&context.local_addr).serve(service).await
    }
}

const INTERVAL: IntervalRequestHandler = IntervalRequestHandler {};
const REPORT: ReportRequestHandler = ReportRequestHandler {};
const EXCESS: ExcessRequestHandler = ExcessRequestHandler {};

static INDEX: &[u8] = b"<p>GET /excess or POST json to /interval or /report</p>";
// 5 MiB
static MAX_CONENT_LENGTH: u32 = 5 << 20;

fn parse_header<T: FromStr>(
    headers: &HeaderMap<HeaderValue>,
    header_name: HeaderName,
) -> Result<T> {
    let parse_error = |e| api_baderr!("Could not parse '{}' header: {}", header_name, e);
    headers
        .get(header_name.clone())
        .ok_or_else(|| api_baderr!("Missing '{}' header", header_name))
        .and_then(|hdr| hdr.to_str().map_err(|e| parse_error(e.to_string())))
        .and_then(|hdr| {
            hdr.parse::<T>()
                .map_err(|_| parse_error("Not a number!".into()))
        })
}

#[async_trait]
pub trait RequestHandler<D, S>
where
    D: DeserializeOwned,
    S: Serialize,
{
    async fn handle(&self, req: D, context: Context) -> std::result::Result<S, ApiError>;
}
fn json_reponse(resp: impl Serialize) -> Result<Response<Body>> {
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&resp)?))?)
}

async fn json_request<D>(req: Request<Body>) -> Result<D>
where
    D: DeserializeOwned,
{
    let content_length: u32 = parse_header(req.headers(), CONTENT_LENGTH)
        .map_err(|e| api_err!(StatusCode::LENGTH_REQUIRED, "{}", e.message))?;
    if content_length > MAX_CONENT_LENGTH {
        return Err(api_err!(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Content too large! Max: {}!",
            MAX_CONENT_LENGTH
        ));
    }
    // mac_address tries to deserialize from borrowed &str
    // (does not work with from_reader)
    //use bytes::Buf;
    //let whole_body = hyper::body::aggregate(req).await?;
    //let value: D = serde_json::from_reader(std::io::BufReader::new(whole_body.reader()))
    //.map_err(|e| api_baderr!("[JSON-Error] {}", e))?;

    let b = to_bytes(req.into_body()).await?;
    serde_json::from_slice(&b).map_err(|e| api_baderr!("[JSON-Error] {}", e))
}

macro_rules! json_resp {
    { $value:expr } => { async move { json_reponse($value.await?) }.await }
}

async fn route_request(
    req: Request<Body>,
    context: Context,
) -> std::result::Result<Response<Body>, GenericError> {
    let uri = req.uri();
    let info_str = format!("[{}] {}", context.remote_addr.unwrap(), uri);
    let resp = match (req.method(), uri.path()) {
        (&Method::POST, "/") | (&Method::GET, "/") | (&Method::GET, "/index.html") => {
            Ok(Response::new(INDEX.into()))
        }
        (&Method::POST, "/interval") => {
            json_resp!(INTERVAL.handle(json_request(req).await?, context))
        }
        (&Method::GET, "/excess") => {
            json_resp!(EXCESS.handle(req.uri().query().unwrap_or("").into(), context))
        }
        (&Method::POST, "/report") => {
            json_resp!(REPORT.handle(json_request(req).await?, context))
        }
        _ => {
            // Return 404 not found response.
            Err(ApiError {
                code: StatusCode::NOT_FOUND,
                message: format!("'{}' Not Found", req.uri().path()),
            }
            .into())
        }
    };
    match resp {
        Ok(r) => {
            info!("{}: OK", info_str);
            Ok(r)
        }
        Err(e) => {
            match e.code {
                StatusCode::INTERNAL_SERVER_ERROR | StatusCode::BAD_GATEWAY => {
                    error!("{}: {}", info_str, e)
                }
                _ => warn!("{}: {}", info_str, e),
            }
            Ok(Response::builder()
                .status(e.code)
                .body(Body::from(
                    // hide wildcard 500 error when not debugging
                    if cfg!(debug_assertions) || e.code != StatusCode::INTERNAL_SERVER_ERROR {
                        e.message
                    } else {
                        "internal server error!".to_string()
                    },
                ))
                .unwrap())
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use hyper::body::Body;
    use hyper::StatusCode;
    use mac_address::MacAddress;

    #[derive(Debug, Serialize, Deserialize)]
    struct RequestMock {
        mac: MacAddress,
        value: String,
    }
    #[derive(PartialEq, Debug, Serialize, Deserialize, Clone)]
    struct ResponseMock {
        mac: MacAddress,
        value: u8,
    }

    struct RequestHandlerMock {
        compare_context: Context,
    }

    #[async_trait]
    impl RequestHandler<RequestMock, ResponseMock> for RequestHandlerMock {
        async fn handle(
            &self,
            req: RequestMock,
            context: Context,
        ) -> std::result::Result<ResponseMock, ApiError> {
            assert_eq!(
                context.remote_addr.unwrap(),
                self.compare_context.remote_addr.unwrap()
            );
            Ok(ResponseMock {
                mac: req.mac,
                value: req
                    .value
                    .parse::<u8>()
                    .map_err(|e| server_err!("Mock response ApiError: {}", e))?,
            })
        }
    }

    fn create_req<T: ToString>(c: T, json: String) -> Request<Body> {
        Request::builder()
            .method(Method::POST)
            .header("Content-Length", c.to_string())
            .body(Body::from(json))
            .unwrap()
    }

    #[tokio::test]
    async fn test_handle_json() {
        let mac = MacAddress::from([0, 0, 0, 0, 0, 0]);
        let req = RequestMock {
            mac: mac.clone(),
            value: "127".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        println!("sending json '{}'", json);

        let bad_req = Request::builder()
            .method(Method::POST)
            .body(Body::from(json.clone()))
            .unwrap();
        assert_matches!(
            json_request::<RequestMock>(bad_req).await,
            Err(e) if e.code == StatusCode::LENGTH_REQUIRED,
            "should require a content-length header"
        );
        assert_matches!(
            json_request::<RequestMock>(create_req("abc", json.clone())).await,
            Err(e) if e.code == StatusCode::LENGTH_REQUIRED,
            "should require valid content-length header"
        );
        assert_matches!(json_request::<RequestMock>(
            create_req(MAX_CONENT_LENGTH + 1, json.clone()),
        )
        .await, Err(e) if e.code == StatusCode::PAYLOAD_TOO_LARGE, "should reject too large content-length");

        let req: RequestMock = json_request(create_req("1000", json.clone()))
            .await
            .unwrap();

        let mut context = Context::load().unwrap();
        context.remote_addr = "127.0.0.1:80".parse().ok();

        let resp = RequestHandlerMock {
            compare_context: context.clone(),
        }
        .handle(req, context)
        .await
        .unwrap();

        assert_eq!(
            resp,
            ResponseMock { mac, value: 127 },
            "should serialize handler response"
        );
    }
}
