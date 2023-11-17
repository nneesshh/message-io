//! Ssl acceptor.

///
pub mod encryption {
    ///
    #[cfg(feature = "native-tls")]
    pub mod native_tls {
        use std::fs::File;
        use std::io::{Read, Write};
        use std::path::PathBuf;

        use native_tls_crate::{HandshakeError as TlsHandshakeError, Identity, TlsAcceptor};

        use crate::adapters::ssl::ssl_stream::SslStream;

        use super::CERT_PATH;
        use super::PRI_KEY_PATH;

        ///
        pub fn create_acceptor(
            cert_path: PathBuf,
            pri_key_path: PathBuf,
        ) -> Result<TlsAcceptor, String> {
            let mut cert_file = File::open(cert_path).unwrap();
            let mut cert = vec![];
            cert_file.read_to_end(&mut cert).unwrap();
            let mut pri_key_file = File::open(pri_key_path).unwrap();
            let mut pri_key = vec![];
            pri_key_file.read_to_end(&mut pri_key).unwrap();

            //
            let pkcs8 = Identity::from_pkcs8(&cert, &pri_key).unwrap();
            TlsAcceptor::new(pkcs8).map_err(|e| e.to_string())
        }

        ///
        pub fn wrap_stream<S>(
            socket: S,
            acceptor: &mut TlsAcceptor,
        ) -> Result<SslStream<S>, io::Error>
        where
            S: Read + Write,
        {
            //
            let accepted = acceptor.accept(socket);
            match accepted {
                Err(e) => {
                    //
                    match e {
                        TlsHandshakeError::Failure(f) => Err(f.to_string()),
                        TlsHandshakeError::WouldBlock(_) => {
                            panic!("Bug: TLS handshake not blocked")
                        }
                    }
                }
                Ok(s) => {
                    //
                    Ok(SslStream::NativeTls(s))
                }
            }
        }
    }

    ///
    #[cfg(feature = "__rustls-tls")]
    pub mod rustls {
        use std::fs::File;
        use std::io;
        use std::io::{BufReader, Read, Write};
        use std::path::PathBuf;
        use std::sync::Arc;

        use pki_types::{CertificateDer, PrivateKeyDer};
        use rustls::server::{Acceptor, ServerConfig};
        use rustls::StreamOwned;

        use crate::adapters::ssl::ssl_stream::SslStream;

        ///
        pub fn create_acceptor() -> Acceptor {
            //
            Acceptor::default()
        }

        ///
        pub fn wrap_stream<S>(
            mut socket: S,
            mut acceptor: Acceptor,
            server_config: Arc<ServerConfig>,
        ) -> Result<SslStream<S>, (io::Error, S, Acceptor, Arc<ServerConfig>)>
        where
            S: Read + Write + Sized,
        {
            // Read TLS packets until we've consumed a full client hello and are ready to accept a
            // connection.
            let accepted = loop {
                match acceptor.read_tls(&mut socket) {
                    Ok(_n) => {
                        //
                        //log::info!("wrap_stream read tls bytes: {}", _n);

                        //
                        match acceptor.accept() {
                            Ok(accepted_opt) => {
                                if let Some(accepted) = accepted_opt {
                                    break accepted;
                                } else {
                                    return Err((
                                        io::Error::new(
                                            io::ErrorKind::Other,
                                            "ClientHelloNotCompletedYet",
                                        ),
                                        socket,
                                        acceptor,
                                        server_config,
                                    ));
                                }
                            }
                            Err(e) => {
                                return Err((
                                    io::Error::new(io::ErrorKind::Other, e.to_string()),
                                    socket,
                                    acceptor,
                                    server_config,
                                ));
                            }
                        }
                    }
                    Err(err) => {
                        return Err((err, socket, acceptor, server_config));
                    }
                }
            };

            //
            match accepted.into_connection(server_config.clone()) {
                Ok(server) => {
                    //
                    let stream = StreamOwned::new(server, socket);
                    Ok(SslStream::RustlsServerConnection(stream))
                }
                Err(err) => {
                    //
                    Err((
                        io::Error::new(io::ErrorKind::Other, err.to_string()),
                        socket,
                        acceptor,
                        server_config,
                    ))
                }
            }
        }

        ///
        pub fn create_server_config(
            cert_path: &PathBuf,
            pri_key_path: &PathBuf,
        ) -> Arc<ServerConfig> {
            //
            let certs = load_certs(&cert_path);
            let pri_key = load_private_key(&pri_key_path);

            let config = ServerConfig::builder()
                .with_safe_defaults()
                .with_no_client_auth()
                .with_single_cert(certs, pri_key)
                .expect("bad certificate/key");

            //
            Arc::new(config)
        }

        fn load_certs(path: &PathBuf) -> Vec<CertificateDer<'static>> {
            let certfile = File::open(path).expect("cannot open certificate file");
            let mut reader = BufReader::new(certfile);
            rustls_pemfile::certs(&mut reader).map(|result| result.unwrap()).collect()
        }

        fn load_private_key(path: &PathBuf) -> PrivateKeyDer<'static> {
            let keyfile = File::open(&path).expect("cannot open private key file");
            let mut reader = BufReader::new(keyfile);

            loop {
                let item_opt =
                    rustls_pemfile::read_one(&mut reader).expect("cannot parse private key file");
                match item_opt {
                    Some(rustls_pemfile::Item::Pkcs1Key(key)) => return key.into(),
                    Some(rustls_pemfile::Item::Pkcs8Key(key)) => return key.into(),
                    Some(rustls_pemfile::Item::Sec1Key(key)) => return key.into(),
                    _ => panic!("no keys found in {:?} (encrypted keys not supported)", path),
                }
            }
        }
    }
}
