use super::*;

fn vm_fiber_step(
    fiber: Rc<RefCell<VmFiber>>,
    continuation: Cont,
    context: crate::core::NativeCallbackContext,
) -> Step {
    let state = fiber.borrow().state();
    match state {
        VmFiberState::Completed(value) => context.with(|| continuation(Ok(value))),
        VmFiberState::Failed(error) => context.with(|| continuation(Err(error.message))),
        VmFiberState::Cancelled => context.with(|| continuation(Err("cancelled".into()))),
        VmFiberState::Running => {
            context.with(|| continuation(Err("bytecode fiber remained running".into())))
        }
        VmFiberState::Suspended => {
            let Some(promise) = fiber.borrow().pending() else {
                return context
                    .with(|| continuation(Err("bytecode fiber suspended without promise".into())));
            };
            Step::Wait(
                promise,
                Box::new(move |state| {
                    context.with(|| {
                        fiber.borrow_mut().resume(state);
                        vm_fiber_step(fiber, continuation, context.clone())
                    })
                }),
            )
        }
        VmFiberState::Yielded(value) => Step::Yield(
            value,
            Box::new(move |resume_value| {
                context.with(|| {
                    fiber.borrow_mut().resume_yield(resume_value);
                    vm_fiber_step(fiber, continuation, context.clone())
                })
            }),
        ),
    }
}

impl Machine {
    pub(super) fn closure_value(program: Rc<Program>, closure: Rc<VmClosure>) -> Value {
        let proto = &program.functions[usize::from(closure.prototype)];
        let arity = usize::from(proto.arity);
        let variadic = proto.variadic;
        let async_function = proto.async_function;
        let name = proto.name.clone();
        let registry = crate::core::namespace_registry().ok();
        let callback_context = crate::core::NativeCallbackContext::capture();
        let fiber_context = callback_context.clone();
        let callback_program = program.clone();
        let callback_closure = closure.clone();
        let callback_registry = registry.clone();
        let callback = move |args: Vec<Value>| {
            let run = || {
                let mut machine = Machine::call_slots(
                    callback_program.clone(),
                    callback_closure.prototype,
                    args.into_iter().map(VmSlot::from).collect(),
                    callback_closure.captures.clone(),
                );
                #[cfg(feature = "tracing-jit")]
                {
                    machine.jit = take_program_jit(&callback_program);
                }
                if async_function {
                    return Ok(Value::Promise(async_result(machine)));
                }
                let outcome = machine.run();
                #[cfg(feature = "tracing-jit")]
                if !matches!(outcome, VmOutcome::Suspended(_) | VmOutcome::Yielded(_)) {
                    store_program_jit(&callback_program, std::mem::take(&mut machine.jit));
                }
                match outcome {
                    VmOutcome::Returned(value) => Ok(value),
                    VmOutcome::Failed(error) => Err(error.message),
                    outcome @ (VmOutcome::Suspended(_) | VmOutcome::Yielded(_)) => {
                        Ok(Value::Promise(async_result_from_outcome(machine, outcome)))
                    }
                }
            };
            callback_context.with(|| match &callback_registry {
                Some(registry) => with_namespace_registry(registry, run),
                None => run(),
            })
        };
        let fiber_callback = move |args: Vec<Value>, continuation: Cont| {
            let start = || {
                let fiber = Rc::new(RefCell::new(VmFiber::start_call(
                    program.clone(),
                    closure.prototype,
                    args,
                    closure
                        .captures
                        .iter()
                        .cloned()
                        .map(|slot| Self::into_value(program.clone(), slot))
                        .collect(),
                )));
                vm_fiber_step(fiber, continuation, fiber_context.clone())
            };
            fiber_context.with(|| match &registry {
                Some(registry) => with_namespace_registry(registry, start),
                None => start(),
            })
        };
        native_fiber_function(
            name.as_deref().unwrap_or("fn"),
            arity,
            variadic,
            callback,
            fiber_callback,
        )
    }
}
