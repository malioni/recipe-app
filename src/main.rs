use recipe_app::network;
use std::net::TcpListener;

fn main() {
    let ip = "127.0.0.1:7878";
    let listener = TcpListener::bind(ip).unwrap();

    for stream in listener.incoming() {
        match stream {
            Ok(stream) =>  { if let Err(e) = network::handle_connection(stream) {
                eprintln!("Issue with the connection: {e:?}")
            }}
            Err(e) => eprintln!("Failed to accept connection: {e:?}"),
        };
    }
}
