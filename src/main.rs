#[macro_use]
extern crate log;

#[macro_use]
extern crate async_trait;
// using the 'async-trait' crate as workaround for passing `async fn` as parameter
// size 'dyn Future' is not known at compile time so Pin<Box<dyn Future<Output = ...>>> is used
// https://rust-lang.github.io/async-book/07_workarounds/04_recursion.html?highlight=BoxFu#recursion

#[macro_use]
extern crate serde;

#[cfg(test)]
#[macro_use]
extern crate assert_matches;

#[macro_use]
mod macros;
mod context;
mod errors;
mod influx_gateway;
mod neighbor;
mod server;
mod wake_heartbeat;
mod interval_handler;
mod excess_handler;
mod report_handler;

#[tokio::main]
async fn main() {
    env_logger::init();
    let context_r = crate::context::Context::load();
    if let Err(e) = context_r {
        error!("Invalid configuration! {}", e);
        panic!();
    }
    // 'context' provides config and state to the request handlers
    let context = context_r.unwrap();
    let wake_heartbeat = wake_heartbeat::wake_heartbeat_loop(context.clone());

    info!("[Informant-Server] {}", context.local_addr);

    use server::{HyperServerWrapper, InformantServer};
    let wrapper = InformantServer::new(context);
    let server = wrapper.serve();
    if let Err(e) = futures::try_join!(server, wake_heartbeat) {
        error!("server error: {}", e);
        panic!();
    }
}
