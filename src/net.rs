use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;
use uefi::proto::network::snp::{ReceiveFlags, SimpleNetwork};
use uefi_raw::MacAddress;

pub struct UefiSnpDevice {
    pub snp: uefi::boot::ScopedProtocol<SimpleNetwork>,
    rx_buffer: alloc::vec::Vec<u8>,
}

impl UefiSnpDevice {
    pub fn new(snp: uefi::boot::ScopedProtocol<SimpleNetwork>) -> Self {
        let _ = snp.start();
        let _ = snp.initialize(0, 0);
        
        let receive_filter = ReceiveFlags::UNICAST | ReceiveFlags::BROADCAST;
        let _ = snp.receive_filters(receive_filter, ReceiveFlags::empty(), true, None);

        Self {
            snp,
            rx_buffer: alloc::vec![0u8; 2048],
        }
    }

    pub fn mac_address(&self) -> [u8; 6] {
        let mac = self.snp.mode().current_address;
        let mut arr = [0u8; 6];
        arr.copy_from_slice(&mac.0[0..6]);
        arr
    }
}

pub struct UefiRxToken {
    buffer: alloc::vec::Vec<u8>,
}

impl RxToken for UefiRxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.buffer)
    }
}

pub struct UefiTxToken<'a> {
    snp: &'a mut uefi::boot::ScopedProtocol<SimpleNetwork>,
}

impl<'a> TxToken for UefiTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = alloc::vec![0u8; len];
        let result = f(&mut buffer);

        match self.snp.transmit(0, &buffer, None, None, None) {
            Ok(_) => {
                loop {
                    if let Ok(Some(recycled)) = self.snp.get_recycled_transmit_buffer_status() {
                        if recycled.as_ptr() == buffer.as_mut_ptr() {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }
            Err(_) => {}
        }

        result
    }
}

impl Device for UefiSnpDevice {
    type RxToken<'a> = UefiRxToken;
    type TxToken<'a> = UefiTxToken<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let mut header_size = 0;
        let mut dest_addr = MacAddress([0; 32]);
        let mut src_addr = MacAddress([0; 32]);
        let mut protocol = 0;

        match self.snp.receive(
            &mut self.rx_buffer,
            Some(&mut header_size),
            Some(&mut src_addr),
            Some(&mut dest_addr),
            Some(&mut protocol),
        ) {
            Ok(size) => {
                let rx_token = UefiRxToken {
                    buffer: self.rx_buffer[..size].to_vec(),
                };
                let tx_token = UefiTxToken {
                    snp: &mut self.snp,
                };
                Some((rx_token, tx_token))
            }
            Err(_) => None,
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(UefiTxToken {
            snp: &mut self.snp,
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = 1500;
        caps.medium = Medium::Ethernet;
        caps
    }
}
