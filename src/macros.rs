#[macro_export(local_inner_macros)]
macro_rules! api_err {
    ( $status:expr, $($fmtarg:expr),+ ) => { crate::errors::ApiError { code: $status, message: std::format!($($fmtarg),+ ) } }
}
#[macro_export(local_inner_macros)]
macro_rules! api_baderr {
    ( $($fmtarg:expr),+ ) => { api_err!(hyper::StatusCode::BAD_REQUEST, $($fmtarg),+ ) }
}
#[macro_export(local_inner_macros)]
macro_rules! server_err {
    ( $($fmtarg:expr),+ ) => { api_err!(hyper::StatusCode::INTERNAL_SERVER_ERROR, $($fmtarg),+ ) }
}
#[macro_export(local_inner_macros)]
macro_rules! fwd_err {
    ( $($fmtarg:expr),+ ) => { api_err!(hyper::StatusCode::BAD_GATEWAY, $($fmtarg),+ ) }
}

//#[macro_export(local_inner_macros)]
//macro_rules! async_boxed_fn {
//{ ($($intype:ty),*) -> $outtype:ty } => { fn ($($intype),*) -> futures::future::BoxFuture<'static, $outtype> }
//}
//#[macro_export(local_inner_macros)]
//macro_rules! def_async_boxed_fn {
//{ $v:vis $name:ident($($inname:ident: $intype:ty),*$(,)?) -> $outtype:ty $body:block } => {
//$v fn $name($($inname: $intype),*) -> futures::future::BoxFuture<'static, $outtype> {
//use futures::future::FutureExt;
//async move { $body }.boxed()
//} } }
