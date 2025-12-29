//! [ArceOS](https://github.com/arceos-org/arceos) Inter-Processor Interrupt (IPI) primitives.

#![cfg_attr(not(test), no_std)]

#[macro_use]
extern crate log;
extern crate alloc;

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

use axhal::{
    irq::{IPI_IRQ, IpiTarget},
    percpu::this_cpu_id,
};
use axtask::AxCpuMask;
use kspin::SpinNoIrq;
use lazyinit::LazyInit;
use queue::IpiEventQueue;

use crate::event::{Callback, MulticastCallback};

mod event;
mod queue;
#[cfg(feature = "smp")]
mod sche;
#[cfg(feature = "smp")]
mod tlb;

static SECONDARY_CPUS_STARTED: AtomicBool = AtomicBool::new(false);

pub fn start_secondary_cpus_done() {
    SECONDARY_CPUS_STARTED.store(true, Ordering::Release);
}

pub fn secondary_cpus_ready() -> bool {
    SECONDARY_CPUS_STARTED.load(Ordering::Acquire)
}

#[percpu::def_percpu]
static IPI_EVENT_QUEUE: LazyInit<SpinNoIrq<IpiEventQueue>> = LazyInit::new();

/// Initialize the per-CPU IPI event queue.
pub fn init() {
    IPI_EVENT_QUEUE.with_current(|ipi_queue| {
        ipi_queue.init_once(SpinNoIrq::new(IpiEventQueue::default()));
    });
}

/// Executes a callback on the specified destination CPU via IPI.
pub fn run_on_cpu<T: Into<Callback>>(name: &'static str, dest_cpu: usize, callback: T, wait: bool) {
    info!("Send IPI event to CPU {}", dest_cpu);
    if dest_cpu == this_cpu_id() {
        // Execute callback on current CPU immediately
        callback.into().call();
    } else {
        let done_flag = if wait {
            Some(Arc::new(AtomicBool::new(false)))
        } else {
            None
        };
        unsafe { IPI_EVENT_QUEUE.remote_ref_raw(dest_cpu) }
            .lock()
            .push(name, this_cpu_id(), callback.into(), done_flag.clone());
        axhal::irq::send_ipi(IPI_IRQ, IpiTarget::Other { cpu_id: dest_cpu });
        if wait {
            if let Some(df) = done_flag {
                while !df.load(Ordering::Acquire) {
                    core::hint::spin_loop();
                }
            }
        }
    }
}

pub fn run_on_bitmask_except_self<T: Into<MulticastCallback>>(
    name: &'static str,
    callback: T,
    cpu_mask: AxCpuMask,
    wait: bool,
) {
    let current_cpu_id = this_cpu_id();
    let cpu_num = axconfig::plat::CPU_NUM;
    let callback = callback.into();

    let mut done_flags: Vec<Arc<AtomicBool>> = Vec::with_capacity(cpu_num - 1);

    for cpu_id in 0..cpu_num {
        if cpu_id != current_cpu_id && cpu_mask.get(cpu_id) {
            let done_flag = if wait {
                Some(Arc::new(AtomicBool::new(false)))
            } else {
                None
            };
            if let Some(df) = &done_flag {
                done_flags.push(df.clone());
            }

            unsafe { IPI_EVENT_QUEUE.remote_ref_raw(cpu_id) }
                .lock()
                .push(
                    name,
                    current_cpu_id,
                    callback.clone().into_unicast(),
                    done_flag,
                );
        }
    }
    for cpu_id in 0..cpu_num {
        if cpu_id != current_cpu_id && cpu_mask.get(cpu_id) {
            axhal::irq::send_ipi(IPI_IRQ, IpiTarget::Other { cpu_id });
        }
    }
    if wait {
        for df in done_flags {
            while !df.load(Ordering::Acquire) {
                core::hint::spin_loop();
            }
        }
    }
}

pub fn run_on_each_cpu_except_self<T: Into<MulticastCallback>>(
    name: &'static str,
    callback: T,
    wait: bool,
) {
    let current_cpu_id = this_cpu_id();
    let cpu_num = axconfig::plat::CPU_NUM;
    let callback = callback.into();

    let mut done_flags: Vec<Arc<AtomicBool>> = Vec::with_capacity(cpu_num - 1);

    // Push the callback to all other CPUs' IPI event queues
    for cpu_id in 0..cpu_num {
        if cpu_id != current_cpu_id {
            let done_flag = if wait {
                Some(Arc::new(AtomicBool::new(false)))
            } else {
                None
            };
            if let Some(df) = &done_flag {
                done_flags.push(df.clone());
            }

            unsafe { IPI_EVENT_QUEUE.remote_ref_raw(cpu_id) }
                .lock()
                .push(
                    name,
                    current_cpu_id,
                    callback.clone().into_unicast(),
                    done_flag,
                );
        }
    }
    // Send IPI to all other CPUs to trigger their callbacks
    axhal::irq::send_ipi(
        IPI_IRQ,
        IpiTarget::AllExceptCurrent {
            cpu_id: current_cpu_id,
            cpu_num,
        },
    );
    if wait {
        for df in done_flags {
            while !df.load(Ordering::Acquire) {
                core::hint::spin_loop();
            }
        }
    }
}

/// Executes a callback on all other CPUs via IPI.
pub fn run_on_each_cpu<T: Into<MulticastCallback>>(name: &'static str, callback: T, wait: bool) {
    info!("Send IPI event to all other CPUs");
    let current_cpu_id = this_cpu_id();
    let cpu_num = axconfig::plat::CPU_NUM;
    let callback = callback.into();

    // Execute callback on current CPU immediately
    callback.clone().call();

    let mut done_flags: Vec<Arc<AtomicBool>> = Vec::with_capacity(cpu_num);
    // Push the callback to all other CPUs' IPI event queues
    for cpu_id in 0..cpu_num {
        if cpu_id != current_cpu_id {
            let done_flag = if wait {
                Some(Arc::new(AtomicBool::new(false)))
            } else {
                None
            };
            if let Some(df) = &done_flag {
                done_flags.push(df.clone());
            }
            unsafe { IPI_EVENT_QUEUE.remote_ref_raw(cpu_id) }
                .lock()
                .push(
                    name,
                    current_cpu_id,
                    callback.clone().into_unicast(),
                    done_flag,
                );
        }
    }
    // Send IPI to all other CPUs to trigger their callbacks
    axhal::irq::send_ipi(
        IPI_IRQ,
        IpiTarget::AllExceptCurrent {
            cpu_id: current_cpu_id,
            cpu_num,
        },
    );
    if wait {
        for df in done_flags {
            while !df.load(Ordering::Acquire) {
                core::hint::spin_loop();
            }
        }
    }
}

/// The handler for IPI events. It retrieves the events from the queue and calls
/// the corresponding callbacks.
pub fn ipi_handler() {
    while let Some((_name, src_cpu_id, callback, done)) =
        unsafe { IPI_EVENT_QUEUE.current_ref_raw() }
            .lock()
            .pop_one()
    {
        debug!("Received IPI event from CPU {src_cpu_id}");
        callback.call();
        if let Some(done) = done {
            done.store(true, Ordering::Release);
        }
    }
}
