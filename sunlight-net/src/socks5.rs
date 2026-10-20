//! SOCKS5 CONNECT with proxy-side hostname resolution and no authentication.
//! Wire format: RFC 1928. The transport must preserve exact read boundaries.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Proxy {
    pub ip: [u8; 4],
    pub port: u16,
}

impl core::str::FromStr for Proxy {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let endpoint = value
            .parse::<core::net::SocketAddrV4>()
            .map_err(|_| "SOCKS proxy must be an IPv4 address and port")?;
        if endpoint.port() == 0 || endpoint.ip().is_unspecified() {
            return Err("SOCKS proxy address and port must be nonzero");
        }
        Ok(Self {
            ip: endpoint.ip().octets(),
            port: endpoint.port(),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Io,
    InvalidDestination,
    Authentication,
    InvalidReply,
    Rejected(u8),
}

pub trait Transport {
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Error>;
    fn read_exact(&mut self, bytes: &mut [u8]) -> Result<(), Error>;
}

pub fn connect<T: Transport>(stream: &mut T, hostname: &str, port: u16) -> Result<(), Error> {
    let name = hostname.as_bytes();
    if name.is_empty()
        || name.len() > 253
        || name.iter().any(|byte| !byte.is_ascii_graphic())
        || port == 0
    {
        return Err(Error::InvalidDestination);
    }
    stream.write_all(&[5, 1, 0])?;
    let mut method = [0; 2];
    stream.read_exact(&mut method)?;
    if method != [5, 0] {
        return Err(Error::Authentication);
    }
    let mut request = [0; 262];
    request[..5].copy_from_slice(&[5, 1, 0, 3, name.len() as u8]);
    request[5..5 + name.len()].copy_from_slice(name);
    request[5 + name.len()..7 + name.len()].copy_from_slice(&port.to_be_bytes());
    stream.write_all(&request[..7 + name.len()])?;
    let mut reply = [0; 4];
    stream.read_exact(&mut reply)?;
    if reply[0] != 5 || reply[2] != 0 {
        return Err(Error::InvalidReply);
    }
    if reply[1] != 0 {
        return Err(Error::Rejected(reply[1]));
    }
    let address_len = match reply[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut length = [0];
            stream.read_exact(&mut length)?;
            if length[0] == 0 {
                return Err(Error::InvalidReply);
            }
            length[0] as usize
        }
        _ => return Err(Error::InvalidReply),
    };
    let mut bound_endpoint = [0; 257];
    stream.read_exact(&mut bound_endpoint[..address_len + 2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    struct Stream {
        input: &'static [u8],
        output: Vec<u8>,
    }

    impl Transport for Stream {
        fn write_all(&mut self, bytes: &[u8]) -> Result<(), Error> {
            self.output.extend_from_slice(bytes);
            Ok(())
        }
        fn read_exact(&mut self, bytes: &mut [u8]) -> Result<(), Error> {
            if self.input.len() < bytes.len() {
                return Err(Error::Io);
            }
            bytes.copy_from_slice(&self.input[..bytes.len()]);
            self.input = &self.input[bytes.len()..];
            Ok(())
        }
    }

    #[test]
    fn sends_hostname_to_proxy_and_preserves_tunnel_bytes() {
        for input in [
            &b"\x05\x00\x05\x00\x00\x01\x7f\x00\x00\x01\x00\x50TLS"[..],
            &b"\x05\x00\x05\x00\x00\x03\x01x\x00\x50TLS"[..],
            &b"\x05\x00\x05\x00\x00\x04\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x01\x00\x50TLS"[..],
        ] {
            let mut stream = Stream {
                input,
                output: Vec::new(),
            };
            connect(&mut stream, "archlinux.org", 443).unwrap();
            assert_eq!(
                stream.output,
                b"\x05\x01\x00\x05\x01\x00\x03\x0darchlinux.org\x01\xbb"
            );
            assert_eq!(stream.input, b"TLS");
        }
    }

    #[test]
    fn rejects_authentication_failure_proxy_failure_and_short_reply() {
        for (input, error) in [
            (&b"\x05\xff"[..], Error::Authentication),
            (&b"\x05\x00\x05\x05\x00\x01"[..], Error::Rejected(5)),
            (&b"\x05\x00\x05\x00\x00\x01"[..], Error::Io),
            (&b"\x05\x00\x05\x00\x00\xff"[..], Error::InvalidReply),
        ] {
            let mut stream = Stream {
                input,
                output: Vec::new(),
            };
            assert_eq!(connect(&mut stream, "archlinux.org", 443), Err(error));
        }
    }

    #[test]
    fn validates_proxy_and_destination_before_io() {
        assert_eq!("10.0.2.2:9050".parse::<Proxy>().unwrap().port, 9050);
        for invalid in ["localhost:9050", "0.0.0.0:9050", "10.0.2.2:0", "10.0.2.2"] {
            assert!(invalid.parse::<Proxy>().is_err());
        }
        let mut stream = Stream {
            input: &[],
            output: Vec::new(),
        };
        assert_eq!(
            connect(&mut stream, "bad\0host", 443),
            Err(Error::InvalidDestination)
        );
        assert!(stream.output.is_empty());
    }
}
