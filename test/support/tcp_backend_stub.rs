use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::thread;

fn handle(mut s: TcpStream, token: &[u8]) {
    let _ = s.write_all(token);
    let _ = s.flush();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: tcp_backend_stub <port> <token>");
        std::process::exit(2);
    }
    let port = &args[1];
    let token = args[2].clone().into_bytes();
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port)).expect("bind failed");
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let token = token.clone();
                thread::spawn(move || handle(s, &token));
            }
            Err(_) => break,
        }
    }
}
