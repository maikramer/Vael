//! Helpers SÓ de teste: um bridge JSON-RPC de mentira.
//!
//! O `vael-tools` fala HTTP à mão (sem dependências); os testes que tocam o
//! bridge usam este servidor de uma resposta por pedido em vez de uma engine.

use std::io::{Read, Write};

use serde_json::{Value, json};

/// Servidor JSON-RPC mínimo: uma resposta por pedido, na ordem dada. O
/// `method` de cada resposta ancora o teste ao método esperado — um pedido
/// fora de ordem falha no servidor (e o cliente acaba no erro de ligação).
pub(crate) fn mock_bridge(replies: Vec<(&'static str, Result<Value, &'static str>)>) -> u16 {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for (method, reply) in replies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = Vec::new();
            let mut header_end = None;
            let mut body_len = 0usize;
            loop {
                if let Some(pos) = header_end
                    && buf.len() >= pos + 4 + body_len
                {
                    break;
                }
                let mut chunk = [0u8; 4096];
                let read = match stream.read(&mut chunk) {
                    Ok(read) if read > 0 => read,
                    _ => break,
                };
                buf.extend_from_slice(&chunk[..read]);
                if header_end.is_none()
                    && let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    body_len = content_length(&buf[..pos]);
                    header_end = Some(pos);
                }
            }
            let request = String::from_utf8_lossy(&buf).into_owned();
            assert!(
                request.contains(&format!("\"method\":\"{method}\"")),
                "método inesperado (esperado `{method}`): {request}"
            );
            let payload = match reply {
                Ok(value) => json!({ "jsonrpc": "2.0", "id": 0, "result": value }),
                Err(message) => json!({
                    "jsonrpc": "2.0",
                    "id": 0,
                    "error": { "code": -32601, "message": message }
                }),
            };
            let body = payload.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    port
}

fn content_length(headers: &[u8]) -> usize {
    String::from_utf8_lossy(headers)
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0)
}
