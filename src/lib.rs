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
use libtransport::{Transport, TransportConfiguration};
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

pub struct TCPtransportCfg<Data> {
    bind_net_addr: String,
    quit_rx: Option<Receiver<()>>,
    listener: TcpListener,
    waker: Option<Waker>,
    phantom: PhantomData<Data>,
}

impl<Data> TransportConfiguration<Data> for TCPtransportCfg<Data> {
    fn new(set_bind_net_addr: String) -> Self {
        let listener = TcpListener::bind(set_bind_net_addr.clone()).unwrap();
        listener
            .set_nonblocking(true)
            .expect("unable to set non-blocking");
        TCPtransportCfg {
            bind_net_addr: set_bind_net_addr,
            quit_rx: None,
            listener,
            waker: None,
            phantom: PhantomData,
        }
    }
    fn set_bind_net_addr(&mut self, address: String) -> Result<()> {
        self.bind_net_addr = address;
        let listener = TcpListener::bind(self.bind_net_addr.clone()).unwrap();
        listener
            .set_nonblocking(true)
            .expect("unable to set non-blocking");
        use std::mem;
        drop(mem::replace(&mut self.listener, listener));
        Ok(())
    }
}

impl<Data> TCPtransportCfg<Data> {
    fn set_quit_rx(&mut self, rx: Receiver<()>) {
        self.quit_rx = Some(rx);
    }
}

//#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TCPtransport<Data> {
    config: Arc<Mutex<TCPtransportCfg<Data>>>,
    quit_tx: Sender<()>,
    server_handle: Option<JoinHandle<()>>,
}

fn listener<Data: 'static>(cfg_mutexed: Arc<Mutex<TCPtransportCfg<Data>>>)
where
    Data: Serialize + DeserializeOwned + Send + Clone,
{
    // FIXME: what we do with unwrap() in threads?
    let config = Arc::clone(&cfg_mutexed);
    loop {
        // check if quit channel got message
        let mut cfg = config.lock().unwrap();
        match &cfg.quit_rx {
            None => {}
            Some(ch) => {
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
    type Configuration = TCPtransportCfg<Data>;

    fn new(mut cfg: Self::Configuration) -> Self {
        let (tx, rx) = mpsc::channel();
        cfg.set_quit_rx(rx);
        let cfg_mutexed = Arc::new(Mutex::new(cfg));
        let config = Arc::clone(&cfg_mutexed);
        let handle = thread::spawn(|| listener(config));
        TCPtransport {
            //            quit_rx: rx,
            quit_tx: tx,
            server_handle: Some(handle),
            config: cfg_mutexed,
        }
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
        lits::common_test::<TCPtransportCfg<lits::Data>, TCPtransport<lits::Data>>(a);
    }
}
