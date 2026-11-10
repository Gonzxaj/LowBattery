//! Simulated HID device for protocol tests (runs on Linux / in Docker).
use crate::hid::HidIo;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;

type Handler = Box<dyn Fn(&[u8]) -> Vec<Vec<u8>>>;

/// Every write/feature write goes through `handler`; its replies are queued
/// as input reports (or as the next feature report).
pub struct FakeHid {
    handler: Handler,
    queue: RefCell<VecDeque<Vec<u8>>>,
}

impl FakeHid {
    pub fn new(handler: impl Fn(&[u8]) -> Vec<Vec<u8>> + 'static) -> Self {
        Self { handler: Box::new(handler), queue: RefCell::default() }
    }
}

impl HidIo for FakeHid {
    fn write(&self, data: &[u8]) -> io::Result<()> {
        self.queue.borrow_mut().extend((self.handler)(data));
        Ok(())
    }
    fn read_timeout(&self, buf: &mut [u8], _: i32) -> io::Result<usize> {
        Ok(self.queue.borrow_mut().pop_front().map_or(0, |r| {
            let n = r.len().min(buf.len());
            buf[..n].copy_from_slice(&r[..n]);
            n
        }))
    }
    fn send_feature(&self, data: &[u8]) -> io::Result<()> {
        self.write(data)
    }
    fn get_feature(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_timeout(buf, 0)
    }
}
