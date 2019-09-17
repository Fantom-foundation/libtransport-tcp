use crate::listener;
use crate::TCPtransportCfg;
use bincode::deserialize;
use buffer::ReadBuffer;
use core::marker::PhantomData;
use futures::stream::Stream;
use futures::task::Context;
use futures::task::Poll;
use libcommon_rs::peer::{Peer, PeerId, PeerList};
use libtransport::errors::Result;
use libtransport::TransportReceiver;
use serde::de::DeserializeOwned;
use std::io;
use std::pin::Pin;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::JoinHandle;

/// Struct which will be implementing the Transport Receiver trait.
pub struct TCPreceiver<Id, Data, E, PL> {
    // The configuration struct (defined in lib.rs)
    config: Arc<Mutex<TCPtransportCfg<Data>>>,
    quit_tx: Sender<()>,
    server_handle: Option<JoinHandle<()>>,
    id: PhantomData<Id>,
    e: PhantomData<E>,
    pl: PhantomData<PL>,
}

/// Specify what occurs when TCPreceiver is dropped.
impl<Id, Data, E, PL> Drop for TCPreceiver<Id, Data, E, PL> {
    fn drop(&mut self) {
        // Send quit message.
        self.quit_tx.send(()).unwrap();
        self.server_handle.take().unwrap().join().unwrap();
    }
}

/// Implementation of the Transport trait.
impl<Id, Pe, Data: 'static, E, PL> TransportReceiver<Id, Data, E, PL>
    for TCPreceiver<Id, Data, E, PL>
where
    Data: DeserializeOwned + Send + Clone,
    Id: PeerId,
    Pe: Peer<Id, E>,
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
        Ok(TCPreceiver {
            //            quit_rx: rx,
            quit_tx: tx,
            server_handle: Some(handle),
            config: cfg_mutexed,
            id: PhantomData,
            e: PhantomData,
            pl: PhantomData,
        })
    }
}

/// Allow TCPreceiver to be store in Pin (for async usage)
impl<Id, D, E, PL> Unpin for TCPreceiver<Id, D, E, PL> {}

/// Implement stream trait to allow TCPreceiver to be used asynchronously. Allows for multiple
/// values to be yielded by a future.
impl<Id, Data, E, PL> Stream for TCPreceiver<Id, Data, E, PL>
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
