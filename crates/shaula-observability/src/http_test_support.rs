pub(crate) async fn serve_one_http_exchange(
    listener: &tokio::net::TcpListener,
    response: &'static [u8],
) -> std::io::Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut socket, _) = listener.accept().await?;
    let mut buf = Vec::new();
    loop {
        let mut chunk = [0u8; 1024];
        let read = socket.read(&mut chunk).await?;
        buf.extend_from_slice(&chunk[..read]);
        let Some(header_end) = buf.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
        let length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .map(|value| value.trim().parse::<usize>().unwrap_or(0))
            .unwrap_or(0);
        if buf.len() >= header_end + 4 + length {
            break;
        }
    }
    socket.write_all(response).await?;
    socket.shutdown().await?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}
