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

/// Specify what occurs when TCPtransport is dropped.
impl<Data> Drop for TCPtransport<Data> {
    fn drop(&mut self) {
        // Send quit message.
        self.quit_tx.send(()).unwrap();
        self.server_handle.take().unwrap().join().unwrap();
    }
}

/// Implementation of the Transport trait.
impl<Id, Pe, Data: 'static, E, PL> Transport<Id, Data, E, PL> for TCPtransport<Data>
where
    Data: Serialize + DeserializeOwned + Send + Clone,
    Id: PeerId,
    Pe: Peer<Id>,
    PL: PeerList<Id, E, P = Pe>,
{
    /// Create a new TCPtransport struct and configure its values.
    fn new(bind_addr: String) -> Result<Self> {
        // Create a new TCPtransport struct.
        let mut cfg = TCPtransportCfg::<Data>::new(bind_addr)?;

        // Create a new Multi Producer Single Consumer FIFO communications channel (with receiver
        // and sender)
        let (tx, rx) = mpsc::channel();
        // Set the config's quit receiver as the returned receiver.
        cfg.set_quit_rx(rx);
        // Wrap config in an Arc<Mutex<>>
        let cfg_mutexed = Arc::new(Mutex::new(cfg));
        // Clone the variable defined above.
        let config = Arc::clone(&cfg_mutexed);
        // Pass the cloned variable to the listener of the server handle.
        let handle = thread::spawn(|| listener(config));
        // Create a new TCPtransport struct and pass cfg_mutexed (original mutex) to the struct.
        Ok(TCPtransport {
            //            quit_rx: rx,
            quit_tx: tx,
            server_handle: Some(handle),
            config: cfg_mutexed,
        })
    }

    /// Sends data to a single, specified peer.
    /// Requires the data to be sent, as well as the net address to be sent too.
    fn send(&mut self, peer_address: String, data: Data) -> Result<()> {
        // Create a TCPstream to the specified address.
        let mut stream = TcpStream::connect(peer_address)?;
        // Serialize data into bytes so that it can be transferred.
        let bytes = serialize(&data)?;
        // Write the byte data and send it through the stream.
        let sent = stream.write(&bytes)?;
        // Check if sent data is same as the serialized data.
        if sent != bytes.len() {
            return Err(Error::Incomplete);
        }
        // Shut down the stream once the message is sent.
        stream.shutdown(std::net::Shutdown::Write)?;
        Ok(())
    }

    /// Send a message to all peers in a PeerList.
    /// Requires a PeerList and data struct.
    fn broadcast(&mut self, peers: &mut PL, data: Data) -> Result<()> {
        // Iterate over all peers
        for p in peers.iter() {
            //dbg!(p.get_net_addr());
            // Create a TCP stream to the current net address.
            let mut stream = TcpStream::connect(p.get_net_addr())?;
            // Serialize data to a bytes.
            let bytes = serialize(&data)?;
            // Write bytes to the stream.
            let sent = stream.write(&bytes)?;
            // Check if sent data is same as the bytes initially made.
            if sent != bytes.len() {
                return Err(Error::Incomplete);
            }
            // Shut down the stream once the message has been sent.
            stream.shutdown(std::net::Shutdown::Write)?;
        }
        Ok(())
    }
}
/// Allow TCPtransport to be store in Pin (for async usage)
impl<D> Unpin for TCPtransport<D> {}

/// Implement stream trait to allow TCPTransport to be used asynchronously. Allows for multiple
/// values to be yielded by a future.
impl<Data> Stream for TCPtransport<Data>
where
    Data: DeserializeOwned,
{
    type Item = Data;
    /// Attempts to resolve the next item in the stream.
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Gets a mutable reference to itself from a Pin value.
        let myself = Pin::get_mut(self);
        // Clones config for later use.
        let config = Arc::clone(&myself.config);
        // Lock config and get the bare mutex guard.
        let mut cfg = config.lock().unwrap();
        // Check the stream in config and process all messages.
        for stream in cfg.listener.incoming() {
            // Check what type of stream is incoming.
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
                // If we get a valid stream
                Ok(mut stream) => {
                    // Create a buffer for the data
                    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
                    loop {
                        // Check to see if we get valid data from the stream.
                        let n = match stream.read_buffer(&mut buffer) {
                            // FIXME: what we do with panics in threads?
                            Err(e) => panic!("error reading from a connection: {}", e),
                            Ok(x) => x.len(),
                        };
                        // Check if the data is not empty.
                        if n == 0 {
                            // FIXME: check correct work in case when TCP next block delivery timeout is
                            // greater than read_buffer() read timeout
                            break;
                        }
                    }
                    // FIXME: what should we return in case of deserialize() failure,
                    // Poll::Ready(None) or Poll::Pending instead of panic?
                    // Deserialize data and package for use.
                    let data: Data = deserialize::<Data>(&buffer).unwrap();
                    // Return a ready status which contains the data.
                    return Poll::Ready(Some(data));
                }
            }
        }
        // Set the config's waker to the context's waker
        cfg.waker = Some(cx.waker().clone());
        // Return a 'Pending' Poll variant
        Poll::Pending
    }
}

/// Tests
#[cfg(test)]
mod tests {
    use super::*;
    extern crate libtransport;
    use libtransport::generic_test as lits;

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
