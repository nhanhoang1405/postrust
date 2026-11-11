use crate::model::{Environment, Request};
use anyhow::{Context, Result, bail};
use reqwest::{
    Client, Method,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use std::time::{Duration, Instant};

pub const RESPONSE_LIMIT: usize = 2 * 1024 * 1024;
#[derive(Debug, Default)]
pub struct Response {
    pub status: String,
    pub headers: String,
    pub body: String,
    pub bytes: usize,
    pub elapsed_ms: u128,
    pub truncated: bool,
}
pub fn client() -> Result<Client> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        // Inspect redirects instead of silently forwarding credentials to another host.
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
pub fn build(client: &Client, request: &Request, env: &Environment) -> Result<reqwest::Request> {
    let method = Method::from_bytes(request.method.as_bytes())?;
    let mut url = reqwest::Url::parse(&env.resolve(&request.url)?)
        .context("Enter an absolute HTTP(S) URL")?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("Only HTTP and HTTPS URLs are supported");
    }
    for line in request
        .params
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let (key, value) = line
            .split_once('=')
            .context("Query parameters use one key=value per line")?;
        url.query_pairs_mut()
            .append_pair(&env.resolve(key.trim())?, &env.resolve(value)?);
    }
    let mut headers = HeaderMap::new();
    for line in request
        .headers
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let (key, value) = line
            .split_once(':')
            .context("Headers use one Name: value per line")?;
        headers.append(
            HeaderName::from_bytes(env.resolve(key.trim())?.as_bytes())
                .context("Invalid header name")?,
            HeaderValue::from_str(&env.resolve(value.trim())?).context("Invalid header value")?,
        );
    }
    let mut builder = client.request(method, url).headers(headers);
    if !request.bearer.trim().is_empty() {
        builder = builder.bearer_auth(env.resolve(request.bearer.trim())?);
    }
    if !request.body.is_empty() {
        builder = builder.body(env.resolve(&request.body)?);
    }
    builder.build().context("Cannot build request")
}
pub async fn execute(client: Client, request: reqwest::Request) -> Result<Response> {
    let started = Instant::now();
    let mut response = client.execute(request).await.context("Request failed")?;
    let status = response.status().to_string();
    let headers = response
        .headers()
        .iter()
        .map(|(key, value)| format!("{key}: {}", String::from_utf8_lossy(value.as_bytes())))
        .collect::<Vec<_>>()
        .join("\n");
    let mut bytes = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = response.chunk().await.context("Reading response failed")? {
        let remaining = RESPONSE_LIMIT - bytes.len();
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if chunk.len() > remaining {
            truncated = true;
            break;
        }
    }
    let body = if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        serde_json::to_string_pretty(&value)?
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok(Response {
        status,
        headers,
        body,
        bytes: bytes.len(),
        elapsed_ms: started.elapsed().as_millis(),
        truncated,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    async fn read_headers(socket: &mut tokio::net::TcpStream) -> String {
        use tokio::io::AsyncReadExt;
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            headers.push(socket.read_u8().await.unwrap());
            assert!(headers.len() < 65536);
        }
        String::from_utf8(headers).unwrap()
    }
    #[test]
    fn new_requests_send_wildcard_accept_and_json_content_type() {
        let request = Request::default();
        let client = client().unwrap();
        let built = build(&client, &request, &Environment::default()).unwrap();
        assert_eq!(built.headers()["accept"], "*/*");
        assert_eq!(built.headers()["content-type"], "application/json");
        let saved: Request = serde_json::from_str(r#"{"headers":"Accept: text/plain"}"#).unwrap();
        assert_eq!(saved.headers, "Accept: text/plain");
    }
    #[test]
    fn builds_encoded_parameters_headers_body_and_auth() {
        let request = Request {
            method: "POST".into(),
            url: "{{base_url}}/echo?existing=yes".into(),
            params: "q=hello & 世界\nq=second".into(),
            headers: "Content-Type: application/json\nX-Test: {{base_url}}".into(),
            body: "{\"url\":\"{{base_url}}\"}".into(),
            bearer: "abc123".into(),
            ..Request::default()
        };
        let built = build(&client().unwrap(), &request, &Environment::default()).unwrap();
        assert_eq!(built.method(), Method::POST);
        assert_eq!(
            built.url().query_pairs().collect::<Vec<_>>()[1].1,
            "hello & 世界"
        );
        assert_eq!(built.headers()["authorization"], "Bearer abc123");
        assert!(
            std::str::from_utf8(built.body().unwrap().as_bytes().unwrap())
                .unwrap()
                .contains("http://localhost:8000")
        );
    }
    #[test]
    fn rejects_invalid_headers_missing_variables_and_non_http_urls() {
        let client = client().unwrap();
        let mut request = Request {
            url: "file:///tmp/file".into(),
            ..Request::default()
        };
        assert!(build(&client, &request, &Environment::default()).is_err());
        request.url = "https://{{missing}}".into();
        assert!(build(&client, &request, &Environment::default()).is_err());
        request.url = "http://localhost".into();
        request.headers = "Broken header".into();
        assert!(build(&client, &request, &Environment::default()).is_err());
    }
    #[tokio::test]
    async fn local_http_server_receives_request_and_error_status_is_a_response() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let incoming = read_headers(&mut socket).await;
            socket.write_all(b"HTTP/1.1 422 Unprocessable Entity\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}").await.unwrap();
            incoming
        });
        let request = Request {
            url: format!("http://{address}/echo"),
            bearer: "token".into(),
            ..Request::default()
        };
        let client = client().unwrap();
        let response = execute(
            client.clone(),
            build(&client, &request, &Environment::default()).unwrap(),
        )
        .await
        .unwrap();
        assert!(response.status.starts_with("422"));
        assert!(response.body.contains("\n  \"ok\": true"));
        assert_eq!(response.bytes, 11);
        assert!(
            server
                .await
                .unwrap()
                .contains("authorization: Bearer token")
        );
    }
    #[tokio::test]
    async fn limits_response_without_waiting_for_unbounded_stream() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                        RESPONSE_LIMIT + 1
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let _ = socket.write_all(&vec![b'x'; RESPONSE_LIMIT + 1]).await;
        });
        let client = client().unwrap();
        let request = Request {
            url: format!("http://{address}"),
            ..Request::default()
        };
        let response = execute(
            client.clone(),
            build(&client, &request, &Environment::default()).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(response.bytes, RESPONSE_LIMIT);
        assert!(response.truncated);
        server.await.unwrap();
    }
}
