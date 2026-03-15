use async_trait::async_trait;
use tokio::net::UdpSocket;
use tracing::debug;

use arp_common::config::ProxyConfig;
use arp_common::crypto::{Compressor, PacketCipher};
use arp_common::transport::prefixed::PrefixedStream;
use arp_common::transport::udp_mux::{read_udp_mux_frame, write_udp_mux_frame, UdpMuxFrame};
use arp_common::transport::MessageTransport;
use arp_common::{Error, Result};

use crate::proxy::ClientProxy;

const UDP_PACKET_MAX: usize = 65535;
const UDP_RESPONSE_TIMEOUT_SECS: u64 = 10;

pub struct UdpProxy {
    name: String,
    local_ip: String,
    local_port: u16,
    use_compression: bool,
    use_encryption: bool,
    secret: String,
}

impl UdpProxy {
    pub fn new(config: ProxyConfig) -> Self {
        let force_secure = config.proxy_type == "sudp";
        Self {
            name: config.name,
            local_ip: config.local_ip,
            local_port: config.local_port,
            use_compression: config.use_compression,
            use_encryption: config.use_encryption || force_secure,
            secret: config.sk,
        }
    }
}

#[async_trait]
impl ClientProxy for UdpProxy {
    async fn handle_work_conn(&self, work_transport: MessageTransport) -> Result<()> {
        let target_addr = format!("{}:{}", self.local_ip, self.local_port);
        let socket = UdpSocket::bind("0.0.0.0:0").await.map_err(Error::Io)?;
        socket.connect(&target_addr).await.map_err(Error::Io)?;
        let (stream, pre_read) = work_transport.into_inner_with_read_buf();
        let mut stream: arp_common::transport::BoxedStream = if pre_read.is_empty() {
            stream
        } else {
            Box::new(PrefixedStream::new(pre_read, stream))
        };

        let mut recv_buf = [0u8; UDP_PACKET_MAX];
        loop {
            let frame = read_udp_mux_frame(&mut stream).await?;
            match frame {
                UdpMuxFrame::Req {
                    request_id,
                    src_addr: _,
                    payload,
                } => {
                    let mut packet = payload;
                    if self.use_encryption {
                        packet = PacketCipher::decrypt(&packet, &self.secret)?;
                    }
                    if self.use_compression {
                        packet = Compressor::decompress(&packet)?;
                    }

                    socket.send(&packet).await.map_err(Error::Io)?;
                    let n = tokio::time::timeout(
                        tokio::time::Duration::from_secs(UDP_RESPONSE_TIMEOUT_SECS),
                        socket.recv(&mut recv_buf),
                    )
                    .await
                    .map_err(|_| Error::Timeout("udp local response timeout".to_string()))?
                    .map_err(Error::Io)?;

                    let mut encoded = recv_buf[..n].to_vec();
                    if self.use_compression {
                        encoded = Compressor::compress(&encoded)?;
                    }
                    if self.use_encryption {
                        encoded = PacketCipher::encrypt(&encoded, &self.secret)?;
                    }
                    write_udp_mux_frame(
                        &mut stream,
                        &UdpMuxFrame::Resp {
                            request_id,
                            payload: encoded,
                        },
                    )
                    .await?;
                    debug!(
                        "UDP proxy {} request={} forwarded={} returned={}",
                        self.name,
                        request_id,
                        packet.len(),
                        n
                    );
                }
                UdpMuxFrame::Ping => {
                    write_udp_mux_frame(&mut stream, &UdpMuxFrame::Pong).await?;
                }
                UdpMuxFrame::Pong => {}
                UdpMuxFrame::Resp { .. } => {}
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}
