use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::vec::Vec;
use alloc::boxed::Box;
use core::cell::RefCell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker, RawWaker, RawWakerVTable};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskId(pub usize);

pub struct Task {
    pub _id: TaskId,
    pub future: RefCell<Pin<Box<dyn Future<Output = ()>>>>,
}

static mut AWAKE_TASKS: Vec<TaskId> = Vec::new();
static mut CURRENT_TASK: Option<TaskId> = None;
static mut GLOBAL_EVENT_REGISTRY: Vec<(uefi::Event, TaskId)> = Vec::new();

unsafe fn clone_waker(data: *const ()) -> RawWaker {
    RawWaker::new(data, &WAKER_VTABLE)
}

unsafe fn wake_waker(data: *const ()) {
    unsafe {
        let task_id = TaskId(data as usize);
        let queue = core::ptr::addr_of_mut!(AWAKE_TASKS);
        if !(*queue).contains(&task_id) {
            (*queue).push(task_id);
        }
    }
}

unsafe fn wake_by_ref_waker(data: *const ()) {
    unsafe {
        wake_waker(data);
    }
}

unsafe fn drop_waker(_data: *const ()) {}

static WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
    clone_waker,
    wake_waker,
    wake_by_ref_waker,
    drop_waker,
);

pub fn waker_for_task(id: TaskId) -> Waker {
    unsafe {
        let raw = RawWaker::new(id.0 as *const (), &WAKER_VTABLE);
        Waker::from_raw(raw)
    }
}

pub fn register_event_for_current_task(event: &uefi::Event) {
    unsafe {
        if let Some(id) = CURRENT_TASK {
            let registry = core::ptr::addr_of_mut!(GLOBAL_EVENT_REGISTRY);
            if !(*registry).iter().any(|(e, tid)| *e == *event && *tid == id) {
                (*registry).push((event.unsafe_clone(), id));
            }
        }
    }
}

pub struct Executor {
    tasks: BTreeMap<TaskId, Rc<Task>>,
    run_queue: Vec<TaskId>,
    event_registry: Vec<(uefi::Event, TaskId)>,
}

impl Executor {
    pub fn new() -> Self {
        Self {
            tasks: BTreeMap::new(),
            run_queue: Vec::new(),
            event_registry: Vec::new(),
        }
    }

    pub fn spawn<F>(&mut self, future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        static mut NEXT_ID: usize = 0;
        let id = unsafe {
            let val = NEXT_ID;
            NEXT_ID += 1;
            TaskId(val)
        };
        let task = Rc::new(Task {
            _id: id,
            future: RefCell::new(Box::pin(future)),
        });
        self.tasks.insert(id, task);
        self.run_queue.push(id);
    }

    pub fn run_once(&mut self) -> bool {
        unsafe {
            let queue = core::ptr::addr_of_mut!(AWAKE_TASKS);
            for id in (*queue).drain(..) {
                if self.tasks.contains_key(&id) && !self.run_queue.contains(&id) {
                    self.run_queue.push(id);
                }
            }
        }

        if self.run_queue.is_empty() {
            return false;
        }

        let run_list = core::mem::take(&mut self.run_queue);
        for id in run_list {
            let task = match self.tasks.get(&id) {
                Some(t) => t.clone(),
                None => continue,
            };

            let waker = waker_for_task(id);
            let mut cx = Context::from_waker(&waker);

            unsafe {
                CURRENT_TASK = Some(id);
            }

            self.event_registry.retain(|(_, tid)| *tid != id);

            let mut future = task.future.borrow_mut();
            let poll_result = future.as_mut().poll(&mut cx);

            unsafe {
                CURRENT_TASK = None;
            }

            unsafe {
                let registry = core::ptr::addr_of_mut!(GLOBAL_EVENT_REGISTRY);
                let mut local_registry = core::mem::take(&mut *registry);
                local_registry.retain(|(event, tid)| {
                    if *tid == id {
                        if let Poll::Pending = poll_result {
                            self.event_registry.push((event.unsafe_clone(), id));
                        }
                        false
                    } else {
                        true
                    }
                });
                *registry = local_registry;
            }

            match poll_result {
                Poll::Ready(()) => {
                    self.tasks.remove(&id);
                    self.event_registry.retain(|(_, tid)| *tid != id);
                }
                Poll::Pending => {}
            }
        }

        true
    }

    pub fn wait_for_events(&mut self) {
        if self.tasks.is_empty() {
            return;
        }

        unsafe {
            let queue = core::ptr::addr_of_mut!(AWAKE_TASKS);
            if !(*queue).is_empty() {
                return;
            }
        }

        let mut events = Vec::new();
        for (event, _) in &self.event_registry {
            events.push(unsafe { event.unsafe_clone() });
        }

        if events.is_empty() {
            uefi::boot::stall(core::time::Duration::from_millis(5));
            return;
        }

        if let Ok(index) = uefi::boot::wait_for_event(events.as_mut_slice()) {
            if index < events.len() {
                let signaled_event = &events[index];
                if let Some((_, task_id)) = self.event_registry.iter().find(|(e, _)| e == signaled_event) {
                    if !self.run_queue.contains(task_id) {
                        self.run_queue.push(*task_id);
                    }
                }
            }
        }
    }

    pub fn run(&mut self) {
        while !self.tasks.is_empty() {
            let ran_any = self.run_once();
            if !ran_any {
                self.wait_for_events();
            }
        }
    }
}
