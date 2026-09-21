//! A direct request must use only the addresses its egress policy checked.
use std::net::SocketAddr;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

#[derive(Debug)]
pub struct PinnedResolver {
    host: String,
    addresses: Vec<SocketAddr>,
}

impl PinnedResolver {
    pub fn new(host: &str, addresses: Vec<SocketAddr>) -> Result<Self, String> {
        let host = normalized_host(host);
        if host.is_empty() || addresses.is_empty() {
            return Err("DNS 固定目标不能为空".into());
        }
        Ok(Self { host, addresses })
    }
}

fn normalized_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

impl Resolve for PinnedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        if normalized_host(name.as_str()) != self.host {
            return Box::pin(async {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "DNS 名称不属于本次已校验的请求目标",
                )
                .into())
            });
        }
        let addresses = self.addresses.clone();
        Box::pin(async move { Ok(Box::new(addresses.into_iter()) as Addrs) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn pins_the_validated_set_and_rejects_other_names() {
        let addresses = vec![
            "8.8.8.8:80".parse().unwrap(),
            "[2606:4700::1111]:80".parse().unwrap(),
        ];
        let resolver = PinnedResolver::new("Pin.Example.", addresses.clone()).unwrap();
        let actual: Vec<_> = resolver
            .resolve("pin.example".parse().unwrap())
            .await
            .unwrap()
            .collect();
        assert_eq!(actual, addresses);
        assert!(resolver
            .resolve("other.example".parse().unwrap())
            .await
            .is_err());
        assert!(PinnedResolver::new("pin.example", Vec::new()).is_err());
    }

    #[tokio::test]
    async fn direct_http_uses_pinned_socket_and_preserves_host_header() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut part = [0; 1024];
                let count = socket.read(&mut part).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&part[..count]);
                if request.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(
                PinnedResolver::new("pin.invalid", vec![address]).unwrap(),
            ))
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let response = client
            .get(format!("http://pin.invalid:{}/", address.port()))
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "ok");
        assert!(server
            .await
            .unwrap()
            .to_ascii_lowercase()
            .contains(&format!("host: pin.invalid:{}\r\n", address.port())));
    }
}
