//! Tls conector.

///
pub mod encryption {
    ///
    pub mod rustls {
        use std::{
            io::{Read, Write},
            sync::Arc,
        };

        use pki_types::ServerName;
        use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

        use crate::adapters::ssl::ssl_stream::SslStream;

        ///
        pub fn wrap_stream<S>(
            socket: S,
            domain: String,
            tls_connector: Option<Arc<ClientConfig>>,
        ) -> Result<SslStream<S>, String>
        where
            S: Read + Write + Sized,
        {
            let config = match tls_connector {
                Some(config) => config,
                None => {
                    #[allow(unused_mut)]
                    let mut roots = RootCertStore::empty();

                    #[cfg(feature = "rustls-tls-native-roots")]
                    {
                        for cert in rustls_native_certs::load_native_certs()
                            .expect("could not load platform certs")
                        {
                            roots.add(cert).unwrap();
                        }
                    }
                    #[cfg(feature = "rustls-tls-webpki-roots")]
                    {
                        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                    }

                    Arc::new(
                        ClientConfig::builder().with_root_certificates(roots).with_no_client_auth(),
                    )
                }
            };
            let domain = ServerName::try_from(domain).map_err(|e| e.to_string())?;
            let client = ClientConnection::new(config, domain).map_err(|e| e.to_string())?;
            let stream = StreamOwned::new(client, socket);

            Ok(SslStream::RustlsClientConnection(stream))
        }
    }
}
