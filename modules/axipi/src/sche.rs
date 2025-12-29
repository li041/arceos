use axhal::percpu::this_cpu_id;
use axtask::RescheIf;

struct RescheImpl;

#[crate_interface::impl_interface]
impl RescheIf for RescheImpl {
    fn send_reschedule_ipi(cpu_id: usize) {
        crate::sche::send_reschedule_ipi(cpu_id);
    }
}

/// Send a reschedule IPI to the specified CPU.
///
/// This is a lightweight IPI that requests the target CPU to check its
/// run queue and reschedule if necessary. The actual scheduling will happen
/// when the target CPU returns from the IPI interrupt.
///
/// # Arguments
/// * `cpu_id` - The target CPU to send the reschedule request to
///
/// # Note
/// This function does nothing if `cpu_id` is the current CPU.
pub fn send_reschedule_ipi(cpu_id: usize) {
    let current_cpu = this_cpu_id();

    // Don't send IPI to self
    if cpu_id == current_cpu {
        return;
    }

    debug!(
        "CPU {} sending reschedule IPI to CPU {}",
        current_cpu, cpu_id
    );

    // The callback will set the preempt_pending flag
    crate::run_on_cpu(
        "reschedule",
        cpu_id,
        || {
            // The reschedule callback: set preempt pending for current task
            let current = axtask::current();
            debug!(
                "CPU {} setting preempt pending for task {}",
                this_cpu_id(),
                current.id_name()
            );
            if !current.is_idle() {
                current.set_preempt_pending(true);
            }
        },
        false, // Don't wait for completion
    );
}
