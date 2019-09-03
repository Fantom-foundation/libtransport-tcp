/// # Fantom Libtransport-tcp
///
/// This library is an asynchronous TCP implementation of the Transport trait in libtransport
/// (https://github.com/Fantom-foundation/libtransport). This library can be tested using the
/// 'common_test' method in libtransport/generic_tests.

extern crate buffer;
extern crate libtransport;
extern crate serde_derive;

use bincode::{deserialize, serialize};
use buffer::ReadBuffer;
use futures::stream::Stream;
use futures::task::Context;
use futures::task::Poll;
use futures::task::Waker;
use libcommon_rs::peer::{Peer, PeerId, PeerList};
use libtransport::errors::{Error, Result};
use libtransport::Transport;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io;
use std::io::Write;
use std::marker::PhantomData;
use std::net::{TcpListener, TcpStream};
use std::pin::Pin;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::JoinHandle;

/// A configuration struct used to hold all TCP data for transport. Keeps track on the message
/// listener, listener address, async waker function, data, and quite receiver.
pub struct TCPtransportCfg<Data> {
    bind_net_addr: String,
    quit_rx: Option<Receiver<()>>,
    listener: TcpListener,
    waker: Option<Waker>,
    phantom: PhantomData<Data>,
}

/// TCPTransportConfig method implementations.
impl<Data> TCPtransportCfg<Data> {
    /// Creates a new config struct and binds the inputted address to a TCP listener.
    /// Requires address to be entered as a String.
    fn new(set_bind_net_addr: String) -> Result<Self> {
        // Create listener and bind address to it.
        let listener = TcpListener::bind(set_bind_net_addr.clone())?;
        // Configure listener.
        listener
            .set_nonblocking(true)
            .expect("unable to set non-blocking");
        // Return config file with net address, listener, and abase phantom data.
        Ok(TCPtransportCfg {
            bind_net_addr: set_bind_net_addr,
            quit_rx: None,
            listener,
            waker: None,
            phantom: PhantomData,
        })
    }
    /// Allows the modification of the net address of the transport config.
    /// Requires address to be added as String.
    fn set_bind_net_addr(&mut self, address: String) -> Result<()> {
        // Set config address to the new address.
        self.bind_net_addr = address;
        // Create a new listener and bind new address to it.
        let listener = TcpListener::bind(self.bind_net_addr.clone()).unwrap();
        // Configure listener
        listener
            .set_nonblocking(true)
            .expect("unable to set non-blocking");
        use std::mem;
        // Replace config listener with new listener and drop old listener.
        drop(mem::replace(&mut self.listener, listener));
        Ok(())
    }
    /// Set the quit receiver to the inputted receiver.
    fn set_quit_rx(&mut self, rx: Receiver<()>) {
        self.quit_rx = Some(rx);
    }
}

//#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// Struct which will be implementing the Transport trait.
/// Requires transportconfig to be wrapped in a mutex + arc combination.
pub struct TCPtransport<Data> {
    // The configuration struct (defined above)
    config: Arc<Mutex<TCPtransportCfg<Data>>>,
    quit_tx: Sender<()>,
    server_handle: Option<JoinHandle<()>>,
}

/// Checks for quit messages, as well as wake method availabilities. If a wake method becomes available,
/// then execute method. If quit is received, then exit loop.
fn listener<Data: 'static>(cfg_mutexed: Arc<Mutex<TCPtransportCfg<Data>>>)
where
    Data: Serialize + DeserializeOwned + Send + Clone,
{
    // FIXME: what we do with unwrap() in threads?
    // Clone the inputted config struct.
    let config = Arc::clone(&cfg_mutexed);
    // Loop continuously, check for wake methods or quit messages.
    loop {
        // check if quit channel got message
        let mut cfg = config.lock().unwrap();
        match &cfg.quit_rx {
            None => {}
            Some(ch) => {
                // If message is successfully received then quit.
                if ch.try_recv().is_ok() {
                    break;
                }
            }
        }
        // allow to pool again if waker is set
        if let Some(waker) = cfg.waker.take() {
            waker.wake()
        }
    }
}

impl<Data> Drop for TCPtransport<Data> {
    fn drop(&mut self) {
        self.quit_tx.send(()).unwrap();
        self.server_handle.take().unwrap().join().unwrap();
    }
}

impl<Id, Pe, Data: 'static, E, PL> Transport<Id, Data, E, PL> for TCPtransport<Data>
where
    Data: Serialize + DeserializeOwned + Send + Clone,
    Id: PeerId,
    Pe: Peer<Id>,
    PL: PeerList<Id, E, P = Pe>,
{
    fn new(bind_addr: String) -> Result<Self> {
        let mut cfg = TCPtransportCfg::<Data>::new(bind_addr)?;
        let (tx, rx) = mpsc::channel();
        cfg.set_quit_rx(rx);
        let cfg_mutexed = Arc::new(Mutex::new(cfg));
        let config = Arc::clone(&cfg_mutexed);
        let handle = thread::spawn(|| listener(config));
        Ok(TCPtransport {
            //            quit_rx: rx,
            quit_tx: tx,
            server_handle: Some(handle),
            config: cfg_mutexed,
        })
    }

    fn send(&mut self, peer_address: String, data: Data) -> Result<()> {
        let mut stream = TcpStream::connect(peer_address)?;
        let bytes = serialize(&data)?;
        let sent = stream.write(&bytes)?;
        if sent != bytes.len() {
            return Err(Error::Incomplete);
        }
        stream.shutdown(std::net::Shutdown::Write)?;
        Ok(())
    }

    fn broadcast(&mut self, peers: &mut PL, data: Data) -> Result<()> {
        for p in peers.iter() {
            //dbg!(p.get_net_addr());
            let mut stream = TcpStream::connect(p.get_net_addr())?;
            let bytes = serialize(&data)?;
            let sent = stream.write(&bytes)?;
            if sent != bytes.len() {
                return Err(Error::Incomplete);
            }
            stream.shutdown(std::net::Shutdown::Write)?;
        }
        Ok(())
    }
}

impl<D> Unpin for TCPtransport<D> {}

impl<Data> Stream for TCPtransport<Data>
where
    Data: DeserializeOwned,
{
    type Item = Data;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let myself = Pin::get_mut(self);
        let config = Arc::clone(&myself.config);
        let mut cfg = config.lock().unwrap();
        for stream in cfg.listener.incoming() {
            match stream {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // check if quit channel got message
                    match &cfg.quit_rx {
                        None => {}
                        Some(ch) => {
                            if ch.try_recv().is_ok() {
                                break; // meaning Poll::Pending as we are going down
                            }
                        }
                    }
                }
                Err(e) => panic!("error in accepting connection: {}", e),
                Ok(mut stream) => {
                    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
                    loop {
                        let n = match stream.read_buffer(&mut buffer) {
                            // FIXME: what we do with panics in threads?
                            Err(e) => panic!("error reading from a connection: {}", e),
                            Ok(x) => x.len(),
                        };
                        if n == 0 {
                            // FIXME: check correct work in case when TCP next block delivery timeout is
                            // greater than read_buffer() read timeout
                            break;
                        }
                    }
                    // FIXME: what should we return in case of deserialize() failure,
                    // Poll::Ready(None) or Poll::Pending instead of panic?
                    let data: Data = deserialize::<Data>(&buffer).unwrap();
                    return Poll::Ready(Some(data));
                }
            }
        }
        cfg.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate libtransport;
    use libtransport::generic_test as lits;

    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }

    #[test]
    fn common() {
        let a: Vec<String> = vec![
            String::from("127.0.0.1:9000"),
            String::from("127.0.0.1:9001"),
            String::from("127.0.0.1:9002"),
        ];
        lits::common_test::<TCPtransport<lits::Data>>(a);
    }
}
