use rtm_core::{Lifecycle, RuntimeEvent};

pub(crate) fn running_event(lifecycle: &Lifecycle) -> RuntimeEvent {
    RuntimeEvent::Running {
        session_id: lifecycle.session_id,
        runtime_pid: lifecycle.runtime_pid,
        start_time: lifecycle.start_time,
    }
}
