extern crate libtransport;
use bincode::serialize;
use core::marker::PhantomData;
use libcommon_rs::peer::{Peer, PeerId, PeerList};
use libtransport::errors::{Error, Result};
use libtransport::TransportSender;
use serde::Serialize;
use std::io::Write;
use std::net::TcpStream;

pub struct TCPsender<Data> {
    phantom: PhantomData<Data>,
}

impl<Id, Pe, Data: 'static, E, PL> TransportSender<Id, Data, E, PL> for TCPsender<Data>
where
    Data: Serialize + Send + Clone,
    Id: PeerId,
    Pe: Peer<Id, E>,
    PL: PeerList<Id, E, P = Pe>,
{
    fn new() -> Result<Self> {
        Ok(TCPsender {
            phantom: PhantomData,
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
            let mut stream = TcpStream::connect(p.get_base_addr())?;
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
    /// Send a message to all peers in a PeerList, on base address.
    /// Requires a PeerList and data struct.
    fn broadcast_n(&mut self, peers: &mut PL, n: usize, data: Data) -> Result<()> {
        // Iterate over all peers
        for p in peers.iter() {
            //dbg!(p.get_net_addr());
            // Create a TCP stream to the current net address.
            let mut stream = TcpStream::connect(p.get_net_addr(n))?;
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
