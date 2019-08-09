extern crate buffer;
extern crate serde_derive;

use bincode::{deserialize, serialize};
use buffer::ReadBuffer;
use libcommon_rs::peer::{Peer, PeerId, PeerList};
use libtransport::errors::{Error, Error::AtMaxVecCapacity, Result};
use libtransport::{Transport, TransportConfiguration};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::JoinHandle;

pub struct TCPtransportCfg<Data> {
    bind_net_addr: String,
    channel_pool: Vec<Sender<Data>>,
    quit_rx: Option<Receiver<()>>,
}

impl<Data> TransportConfiguration<Data> for TCPtransportCfg<Data> {
    fn register_channel(&mut self, sender: Sender<Data>) -> Result<()> {
        // Vec::push() panics when number of elements overflows `usize`
        if self.channel_pool.len() == std::usize::MAX {
            return Err(AtMaxVecCapacity);
        }
        self.channel_pool.push(sender);
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

fn handle_client<D: Clone>(cfg_mutexed: Arc<Mutex<TCPtransportCfg<D>>>, mut stream: TcpStream)
where
    D: DeserializeOwned,
{
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
    let data: D = deserialize::<D>(&buffer).unwrap();
    let cfg = cfg_mutexed.lock().unwrap();
    for ch in cfg.channel_pool.iter() {
        ch.send(data.clone()).unwrap();
    }
}

fn listener<Data: 'static>(cfg_mutexed: Arc<Mutex<TCPtransportCfg<Data>>>)
where
    Data: AsRef<u8> + Serialize + DeserializeOwned + Send + Clone,
{
    // FIXME: what we do with unwrap() in threads?
    let config = Arc::clone(&cfg_mutexed);
    let cfg = config.lock().unwrap();
    let listener = TcpListener::bind(cfg.bind_net_addr.clone()).unwrap();
    listener
        .set_nonblocking(true)
        .expect("unable to set non-blocking");
    for stream in listener.incoming() {
        match stream {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // check if quit channel got message
                match &cfg.quit_rx {
                    None => {}
                    Some(ch) => {
                        if ch.try_recv().is_ok() {
                            break;
                        }
                    }
                }
                continue;
            }
            Err(e) => panic!("error in accepting connection: {}", e),
            Ok(stream) => {
                let config = Arc::clone(&cfg_mutexed);
                // receive Data and push it into channels, pipes and call callbacks
                thread::spawn(move || handle_client(config, stream));
            }
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
    Data: AsRef<u8> + Serialize + DeserializeOwned + Send + Clone,
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

    fn register_channel(&mut self, sender: Sender<Data>) -> Result<()> {
        let mut cfg = self.config.lock()?;
        cfg.register_channel(sender)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
