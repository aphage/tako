use std::io;

use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};

pub fn create_server(path: &str, first_instance: bool) -> io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first_instance)
        .reject_remote_clients(true);
    options.create(path)
}

pub fn open_client(path: &str) -> io::Result<NamedPipeClient> {
    ClientOptions::new().open(path)
}
