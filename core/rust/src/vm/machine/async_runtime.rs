//! Cooperative promise scheduling for bytecode machines.

use super::*;

impl Machine {
    fn retain_async_child(
        scheduler: &Rc<RefCell<AsyncScheduler>>,
        machine: Machine,
        result: Promise,
        pending: Promise,
    ) {
        let id = {
            let mut state = scheduler.borrow_mut();
            let id = state.next_id;
            state.next_id = id.wrapping_add(1);
            state.children.insert(
                id,
                AsyncChild {
                    machine,
                    result: result.downgrade(),
                    pending: pending.clone(),
                },
            );
            id
        };
        let weak = Rc::downgrade(scheduler);
        pending.on_settle(Rc::new(move |state| {
            if let Some(scheduler) = weak.upgrade() {
                scheduler.borrow_mut().ready.push_back((id, state));
            }
        }));
    }

    fn finish_async(
        scheduler: &Rc<RefCell<AsyncScheduler>>,
        mut machine: Machine,
        result: Promise,
        outcome: VmOutcome,
    ) {
        match outcome {
            VmOutcome::Returned(value) => {
                #[cfg(feature = "tracing-jit")]
                store_program_jit(&machine.program.clone(), std::mem::take(&mut machine.jit));
                settle_result(&result, Ok(value));
            }
            VmOutcome::Failed(error) => {
                #[cfg(feature = "tracing-jit")]
                store_program_jit(&machine.program.clone(), std::mem::take(&mut machine.jit));
                result.reject(error.message);
            }
            VmOutcome::Suspended(pending) => {
                Self::retain_async_child(scheduler, machine, result, pending)
            }
            VmOutcome::Yielded(_) => {
                #[cfg(feature = "tracing-jit")]
                store_program_jit(&machine.program.clone(), std::mem::take(&mut machine.jit));
                result.reject("coroutine/yield used outside of a coroutine");
            }
        }
    }

    fn poll_scheduler(scheduler: &Rc<RefCell<AsyncScheduler>>) -> usize {
        {
            let mut state = scheduler.borrow_mut();
            if state.polling {
                return 0;
            }
            state.polling = true;
        }
        let pending = scheduler
            .borrow()
            .children
            .values()
            .map(|child| child.pending.clone())
            .collect::<Vec<_>>();
        for promise in pending {
            promise.state();
        }
        let mut count = 0;
        loop {
            let Some((id, state)) = scheduler.borrow_mut().ready.pop_front() else {
                break;
            };
            let Some(mut child) = scheduler.borrow_mut().children.remove(&id) else {
                continue;
            };
            let Some(result) = child.result.upgrade() else {
                child.pending.cancel();
                continue;
            };
            count += 1;
            let outcome = child.machine.resume(state);
            Self::finish_async(scheduler, child.machine, result, outcome);
        }
        scheduler.borrow_mut().polling = false;
        count
    }

    fn cancel_async_result(scheduler: &Rc<RefCell<AsyncScheduler>>, identity: usize) {
        let ids = scheduler
            .borrow()
            .children
            .iter()
            .filter_map(|(id, child)| {
                child
                    .result
                    .upgrade()
                    .is_some_and(|candidate| candidate.identity_address() == identity)
                    .then_some(*id)
            })
            .collect::<Vec<_>>();
        for id in ids {
            let child = { scheduler.borrow_mut().children.remove(&id) };
            if let Some(child) = child {
                child.pending.cancel();
            }
        }
    }

    pub(super) fn spawn_async(&self, mut machine: Machine) -> Promise {
        let scheduler = self
            .scheduler
            .upgrade()
            .expect("root VM owns its async scheduler");
        machine.scheduler = Rc::downgrade(&scheduler);
        machine.scheduler_owner = None;
        let result = Promise::new();
        let context = crate::core::NativeCallbackContext::capture();
        let poll = scheduler.clone();
        let poll_context = context.clone();
        result.set_poller(Rc::new(move || {
            poll_context.with(|| Self::poll_scheduler(&poll));
        }));
        let wait = scheduler.clone();
        let wait_context = context.clone();
        result.set_waiter(Rc::new(move || {
            wait_context.with(|| Self::poll_scheduler(&wait));
        }));
        let cancel_scheduler = scheduler.clone();
        let cancel_identity = result.identity_address();
        result.set_cancel_hook(Rc::new(move || {
            Self::cancel_async_result(&cancel_scheduler, cancel_identity);
        }));
        let outcome = machine.run();
        Self::finish_async(&scheduler, machine, result.clone(), outcome);
        result
    }

    pub fn poll_async(&mut self) -> usize {
        self.scheduler
            .upgrade()
            .map(|scheduler| Self::poll_scheduler(&scheduler))
            .unwrap_or(0)
    }
}

pub(super) fn async_result(mut machine: Machine) -> Promise {
    let outcome = machine.run();
    async_result_from_outcome(machine, outcome)
}

pub(super) fn async_result_from_outcome(mut machine: Machine, outcome: VmOutcome) -> Promise {
    let scheduler = machine
        .scheduler
        .upgrade()
        .or_else(|| machine.scheduler_owner.clone())
        .expect("VM owns its async scheduler");
    machine.scheduler = Rc::downgrade(&scheduler);
    machine.scheduler_owner = None;
    let result = Promise::new();
    let context = crate::core::NativeCallbackContext::capture();
    let poll = scheduler.clone();
    let poll_context = context.clone();
    result.set_poller(Rc::new(move || {
        poll_context.with(|| Machine::poll_scheduler(&poll));
    }));
    let wait = scheduler.clone();
    let wait_context = context.clone();
    result.set_waiter(Rc::new(move || {
        wait_context.with(|| Machine::poll_scheduler(&wait));
    }));
    let cancel_scheduler = scheduler.clone();
    let cancel_identity = result.identity_address();
    result.set_cancel_hook(Rc::new(move || {
        Machine::cancel_async_result(&cancel_scheduler, cancel_identity);
    }));
    Machine::finish_async(&scheduler, machine, result.clone(), outcome);
    result
}
