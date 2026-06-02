use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use crate::executor::register_event_for_current_task;

pub struct NetworkSleepFuture {
    timer_event: Option<uefi::Event>,
    is_started: bool,
}

impl NetworkSleepFuture {
    pub fn new() -> Self {
        Self {
            timer_event: None,
            is_started: false,
        }
    }
}

impl Drop for NetworkSleepFuture {
    fn drop(&mut self) {
        let _ = self.timer_event.take();
    }
}

#[cfg(not(feature = "wait-packet-workaround"))]
fn get_packet_wait_event() -> Option<uefi::Event> {
    unsafe {
        let net_ptr = core::ptr::addr_of_mut!(crate::GLOBAL_NET);
        let net = (*net_ptr).as_ref()?;
        net.device.snp.wait_for_packet_event().ok()
    }
}

impl Future for NetworkSleepFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.is_started {
            return Poll::Ready(());
        }
        self.is_started = true;

        #[cfg(not(feature = "wait-packet-workaround"))]
        if let Some(packet_event) = get_packet_wait_event() {
            register_event_for_current_task(&packet_event);
        }

        unsafe {
            match uefi::boot::create_event(
                uefi::boot::EventType::TIMER,
                uefi::boot::Tpl::APPLICATION,
                None,
                None,
            ) {
                Ok(timer) => {
                    let _ = uefi::boot::set_timer(&timer, uefi::boot::TimerTrigger::Relative(100_000));
                    register_event_for_current_task(&timer);
                    self.timer_event = Some(timer);
                }
                Err(_) => {}
            }
        }

        Poll::Pending
    }
}

pub struct SlintSleepFuture {
    timer_event: Option<uefi::Event>,
    is_started: bool,
    duration: Duration,
}

impl SlintSleepFuture {
    pub fn new(duration: Duration) -> Self {
        Self {
            timer_event: None,
            is_started: false,
            duration,
        }
    }
}

impl Drop for SlintSleepFuture {
    fn drop(&mut self) {
        let _ = self.timer_event.take();
    }
}

impl Future for SlintSleepFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.is_started {
            return Poll::Ready(());
        }
        self.is_started = true;

        if let Ok(key_event) = uefi::system::with_stdin(|stdin| stdin.wait_for_key_event()) {
            register_event_for_current_task(&key_event);
        }

        unsafe {
            let pointers = &mut *core::ptr::addr_of_mut!(crate::slint_plat::MOUSE_POINTERS);
            for mpointer in pointers.iter_mut() {
                if let Ok(event) = mpointer.wait_for_input_event() {
                    register_event_for_current_task(&event);
                }
            }
        }

        let ticks = (self.duration.as_nanos() / 100) as u64;
        if ticks > 0 {
            unsafe {
                match uefi::boot::create_event(
                    uefi::boot::EventType::TIMER,
                    uefi::boot::Tpl::APPLICATION,
                    None,
                    None,
                ) {
                    Ok(timer) => {
                        let _ = uefi::boot::set_timer(&timer, uefi::boot::TimerTrigger::Relative(ticks));
                        register_event_for_current_task(&timer);
                        self.timer_event = Some(timer);
                    }
                    Err(_) => {}
                }
            }
        }

        Poll::Pending
    }
}

pub struct TimerSleepFuture {
    timer_event: Option<uefi::Event>,
    is_started: bool,
    duration: Duration,
}

impl TimerSleepFuture {
    pub fn new(duration: Duration) -> Self {
        Self {
            timer_event: None,
            is_started: false,
            duration,
        }
    }
}

impl Drop for TimerSleepFuture {
    fn drop(&mut self) {
        let _ = self.timer_event.take();
    }
}

impl Future for TimerSleepFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.is_started {
            return Poll::Ready(());
        }
        self.is_started = true;

        let ticks = (self.duration.as_nanos() / 100) as u64;
        if ticks > 0 {
            unsafe {
                match uefi::boot::create_event(
                    uefi::boot::EventType::TIMER,
                    uefi::boot::Tpl::APPLICATION,
                    None,
                    None,
                ) {
                    Ok(timer) => {
                        let _ = uefi::boot::set_timer(&timer, uefi::boot::TimerTrigger::Relative(ticks));
                        register_event_for_current_task(&timer);
                        self.timer_event = Some(timer);
                    }
                    Err(_) => {}
                }
            }
        }

        Poll::Pending
    }
}

