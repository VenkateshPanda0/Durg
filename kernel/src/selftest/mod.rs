//! In-kernel self-tests, run inside QEMU with the `selftest` feature.
//!
//! Rust's built-in test framework needs nightly features for a bare-metal
//! target (ADR-0002 keeps us on stable), so this is a small registry of our
//! own: each test is a function that panics on failure. The panic handler
//! prints the message and exits QEMU with the failure code, so the host-side
//! harness sees both which test failed and a non-zero status. A test that
//! hangs is caught by the host-side timeout.
//!
//! The `fail-demo` feature adds a test that always fails. `cargo xtask test`
//! boots that variant too and requires it to be reported as a failure, which
//! proves the harness can fail.

use durg_paging::{flags, PageSize, WalkError};

use crate::acpi;
use crate::arch::serial;
use crate::arch::x86_64::{self, gdt, idt, lapic, pic, trap};
use crate::cspace::{self, CSpace};
use crate::limine::{self, MemoryKind};
use crate::mm;
use crate::mm::frame;
use crate::mm::paging::{self, AddressSpace, UnmapError};
use crate::mm::vm;
use crate::qemu::{self, ExitCode};
use crate::sched;
use crate::sched::ExitStatus as user_status;
use crate::user;
use durg_syscall::{derive_label, label, retype_arg, Error as SysError};

mod capabilities;
mod early;
mod elf_tests;
mod heap_tests;
mod messaging;
mod threads;
mod usermode;

use self::capabilities::*;
use self::early::*;
use self::elf_tests::*;
use self::heap_tests::*;
use self::messaging::*;
use self::threads::*;
use self::usermode::*;

struct Test {
    name: &'static str,
    run: fn(),
}

const PAGE_SIZE: u64 = 4096;

const TESTS: &[Test] = &[
    Test {
        name: "serial_loopback",
        run: serial_loopback,
    },
    Test {
        name: "memory_map_has_usable_ram",
        run: memory_map_has_usable_ram,
    },
    Test {
        name: "memory_map_sorted_by_base",
        run: memory_map_sorted_by_base,
    },
    Test {
        name: "usable_regions_aligned_and_disjoint",
        run: usable_regions_aligned_and_disjoint,
    },
    Test {
        name: "hhdm_offset_in_higher_half",
        run: hhdm_offset_in_higher_half,
    },
    Test {
        name: "usable_ram_readable_through_hhdm",
        run: usable_ram_readable_through_hhdm,
    },
    Test {
        name: "base_revision_4_loaded",
        run: base_revision_4_loaded,
    },
    Test {
        name: "rsdp_provided_and_virtual",
        run: rsdp_provided_and_virtual,
    },
    Test {
        name: "madt_discovered",
        run: madt_discovered,
    },
    Test {
        name: "cpu_has_local_apic",
        run: cpu_has_local_apic,
    },
    Test {
        name: "gdt_and_tss_loaded",
        run: gdt_and_tss_loaded,
    },
    Test {
        name: "gdt_has_correct_user_segments",
        run: gdt_has_correct_user_segments,
    },
    Test {
        name: "idt_loaded",
        run: idt_loaded,
    },
    Test {
        name: "legacy_pic_masked",
        run: legacy_pic_masked,
    },
    Test {
        name: "breakpoint_exception_returns",
        run: breakpoint_exception_returns,
    },
    Test {
        name: "page_fault_is_caught_with_cr2_and_error_code",
        run: page_fault_is_caught,
    },
    Test {
        name: "hhdm_probe_does_not_fault",
        run: hhdm_probe_does_not_fault,
    },
    Test {
        name: "paging_translates_hhdm_to_physical",
        run: paging_translates_hhdm_to_physical,
    },
    Test {
        name: "paging_translates_kernel_code_into_the_kernel_region",
        run: paging_translates_kernel_code,
    },
    Test {
        name: "paging_reports_unmapped_addresses",
        run: paging_reports_unmapped_addresses,
    },
    Test {
        name: "pat_entry_3_is_uncacheable",
        run: pat_entry_3_is_uncacheable,
    },
    Test {
        name: "lapic_is_mapped_uncached_at_its_physical_base",
        run: lapic_is_mapped_uncached,
    },
    Test {
        name: "lapic_registers_read_back_sane_values",
        run: lapic_registers_read_back,
    },
    Test {
        name: "lapic_base_matches_the_madt",
        run: lapic_base_matches_the_madt,
    },
    Test {
        name: "map_page_refuses_to_replace_a_mapping",
        run: map_page_refuses_to_replace_a_mapping,
    },
    Test {
        name: "pmm_totals_match_an_independent_calculation",
        run: pmm_totals_match_an_independent_calculation,
    },
    Test {
        name: "pmm_hands_out_only_usable_distinct_aligned_frames",
        run: pmm_hands_out_only_usable_distinct_frames,
    },
    Test {
        name: "pmm_rejects_double_free_and_foreign_frames",
        run: pmm_rejects_double_free_and_foreign_frames,
    },
    Test {
        name: "pmm_zeroed_frames_are_zero",
        run: pmm_zeroed_frames_are_zero,
    },
    Test {
        name: "pmm_stress_with_unique_patterns",
        run: pmm_stress_with_unique_patterns,
    },
    Test {
        name: "pmm_exhaustion_and_recovery",
        run: pmm_exhaustion_and_recovery,
    },
    Test {
        name: "pmm_block_return_is_atomic",
        run: pmm_block_return_is_atomic,
    },
    Test {
        name: "map_page_takes_new_table_frames_from_the_pmm",
        run: map_page_takes_new_table_frames_from_the_pmm,
    },
    Test {
        name: "vm_kernel_page_tables_are_active",
        run: vm_kernel_page_tables_are_active,
    },
    Test {
        name: "vm_no_kernel_page_is_both_writable_and_executable",
        run: vm_no_page_is_writable_and_executable,
    },
    Test {
        name: "vm_sections_have_the_intended_permissions",
        run: vm_sections_have_the_intended_permissions,
    },
    Test {
        name: "vm_writing_read_only_data_faults",
        run: vm_writing_read_only_data_faults,
    },
    Test {
        name: "vm_executing_data_faults_on_an_instruction_fetch",
        run: vm_executing_data_faults,
    },
    Test {
        name: "vm_kernel_image_has_no_alias_in_the_direct_map",
        run: vm_kernel_image_has_no_alias_in_the_direct_map,
    },
    Test {
        name: "vm_framebuffer_and_reserved_memory_are_not_mapped",
        run: vm_framebuffer_and_reserved_memory_are_not_mapped,
    },
    Test {
        name: "vm_running_on_the_kernel_stack_with_a_guard_page",
        run: vm_running_on_the_kernel_stack_with_a_guard_page,
    },
    Test {
        name: "paging_map_and_unmap_round_trip",
        run: paging_map_and_unmap_round_trip,
    },
    Test {
        name: "paging_unmap_takes_effect_despite_the_tlb",
        run: paging_unmap_takes_effect_despite_the_tlb,
    },
    Test {
        name: "paging_unmap_errors",
        run: paging_unmap_errors,
    },
    Test {
        name: "paging_2m_page_maps_translates_and_unmaps",
        run: paging_2m_page_maps_translates_and_unmaps,
    },
    Test {
        name: "paging_map_range_mixes_page_sizes",
        run: paging_map_range_mixes_page_sizes,
    },
    Test {
        name: "heap_boxes_vecs_strings_and_maps_work",
        run: heap_collections_work,
    },
    Test {
        name: "heap_alignment_up_to_a_page_is_honoured",
        run: heap_alignment_is_honoured,
    },
    Test {
        name: "heap_random_alloc_free_stress_keeps_contents_intact",
        run: heap_stress_keeps_contents_intact,
    },
    Test {
        name: "heap_grows_on_demand_and_stays_consistent",
        run: heap_grows_on_demand,
    },
    Test {
        name: "heap_pages_are_writable_and_not_executable",
        run: heap_pages_are_rw_nx,
    },
    Test {
        name: "heap_refused_request_fails_gracefully_and_heap_stays_usable",
        run: heap_oom_is_graceful,
    },
    Test {
        name: "heap_detects_double_and_wild_frees",
        run: heap_detects_bad_frees,
    },
    Test {
        name: "irq_lock_masks_real_interrupts_and_restores_them",
        run: irq_lock_masks_real_interrupts,
    },
    Test {
        name: "sched_unstarted_spawn_releases_owned_spaces",
        run: sched_unstarted_spawn_releases_owned_spaces,
    },
    Test {
        name: "sched_spawn_join_runs_a_thread_and_returns_its_resources",
        run: sched_spawn_join_returns_resources,
    },
    Test {
        name: "sched_yield_alternates_between_cooperating_threads",
        run: sched_yield_alternates,
    },
    Test {
        name: "sched_switch_preserves_callee_saved_registers",
        run: sched_switch_preserves_registers,
    },
    Test {
        name: "sched_timer_preempts_threads_that_never_yield",
        run: sched_timer_preempts,
    },
    Test {
        name: "sched_irq_lock_prevents_lost_updates_under_preemption",
        run: sched_lock_prevents_lost_updates,
    },
    Test {
        name: "sched_thread_stacks_have_unmapped_guard_pages",
        run: sched_thread_stack_guard_pages,
    },
    Test {
        name: "sched_refuses_threads_beyond_the_limit_and_recovers",
        run: sched_thread_limit,
    },
    Test {
        name: "user_space_shares_the_kernel_half_and_has_an_empty_lower_half",
        run: user_space_shares_kernel_half,
    },
    Test {
        name: "user_mappings_are_user_accessible_at_every_level_kernel_ones_never",
        run: user_bit_at_every_level,
    },
    Test {
        name: "user_accessible_page_in_the_kernel_half_is_refused",
        run: user_page_in_kernel_half_refused,
    },
    Test {
        name: "kernel_mappings_made_later_appear_in_existing_user_spaces",
        run: later_kernel_mappings_are_shared,
    },
    Test {
        name: "switching_to_a_user_space_keeps_the_kernel_running",
        run: switching_address_spaces_works,
    },
    Test {
        name: "destroying_a_user_space_returns_every_frame",
        run: destroy_user_space_returns_frames,
    },
    Test {
        name: "syscall_msrs_are_programmed",
        run: user_msrs_programmed,
    },
    Test {
        name: "user_program_runs_in_ring_3_and_calls_the_kernel",
        run: user_hello_round_trip,
    },
    Test {
        name: "user_faults_end_only_the_task",
        run: user_faults_end_only_the_task,
    },
    Test {
        name: "user_bad_system_calls_are_refused_and_the_task_goes_on",
        run: user_bad_system_calls,
    },
    Test {
        name: "user_output_is_sanitised_before_it_reaches_the_console",
        run: user_output_is_sanitised,
    },
    Test {
        name: "user_registers_survive_system_calls",
        run: user_registers_survive_syscalls,
    },
    Test {
        name: "user_task_starts_with_every_register_zeroed",
        run: user_starts_with_zeroed_registers,
    },
    Test {
        name: "user_task_is_preempted_in_ring_3_and_can_be_killed",
        run: user_preempted_and_killed,
    },
    Test {
        name: "blocked_user_ipc_tasks_can_be_killed",
        run: blocked_user_ipc_tasks_can_be_killed,
    },
    Test {
        name: "user_tasks_return_all_their_resources",
        run: user_tasks_return_resources,
    },
    Test {
        name: "syscall_handler_survives_random_arguments",
        run: syscall_handler_survives_random_arguments,
    },
    Test {
        name: "cpu_protections_match_what_cpuid_reports",
        run: cpu_protections_match_cpuid,
    },
    Test {
        name: "smep_stops_the_kernel_executing_user_pages",
        run: smep_blocks_kernel_execute_of_user_pages,
    },
    Test {
        name: "smap_stops_the_kernel_touching_user_pages_by_accident",
        run: smap_blocks_accidental_user_access,
    },
    Test {
        name: "umip_stops_ring_3_reading_descriptor_table_addresses",
        run: umip_blocks_sidt_in_ring_3,
    },
    Test {
        name: "boot_modules_are_present_and_parse_as_elf",
        run: boot_modules_parse_as_elf,
    },
    Test {
        name: "elf_loader_gives_each_page_exactly_its_segments_permissions",
        run: elf_loader_permissions,
    },
    Test {
        name: "root_task_runs_from_its_elf_module",
        run: root_task_runs_from_module,
    },
    Test {
        name: "task_a_and_task_b_cannot_read_each_others_memory",
        run: tasks_cannot_read_each_others_memory,
    },
    Test {
        name: "frames_freed_by_one_task_are_zeroed_for_the_next",
        run: freed_frames_leave_no_residue,
    },
    Test {
        name: "boot_module_memory_is_read_only_and_not_executable",
        run: module_window_is_read_only,
    },
    Test {
        name: "a_task_that_can_see_the_others_secret_notices",
        run: leak_detector_detects_a_leak,
    },
    Test {
        name: "cspace_starts_with_only_a_capability_to_itself",
        run: cspace_starts_with_only_a_capability_to_itself,
    },
    Test {
        name: "retyped_memory_is_zeroed_and_only_that_memory",
        run: retyped_memory_is_zeroed,
    },
    Test {
        name: "capability_errors_change_nothing_and_rights_never_grow",
        run: capability_errors_change_nothing,
    },
    Test {
        name: "destroying_a_cspace_returns_every_frame_and_block",
        run: destroying_a_cspace_returns_everything,
    },
    Test {
        name: "invoke_survives_random_arguments_and_keeps_every_invariant",
        run: invoke_survives_random_arguments,
    },
    Test {
        name: "capabilities_work_from_ring_3",
        run: capabilities_work_from_ring_3,
    },
    Test {
        name: "a_task_without_a_cspace_can_name_nothing",
        run: a_task_without_a_cspace_can_name_nothing,
    },
    Test {
        name: "the_root_task_starts_with_exactly_its_memory",
        run: the_root_task_starts_with_its_memory,
    },
    Test {
        name: "notification_ring3_blocked_delivery_and_revocation",
        run: notification_ring3_blocked_delivery_and_revocation,
    },
    Test {
        name: "notifications_coalesce_wake_fifo_and_abort",
        run: notifications_coalesce_wake_fifo_and_abort,
    },
    Test {
        name: "ipc_a_sender_waits_for_a_receiver_and_a_receiver_for_a_sender",
        run: ipc_rendezvous_in_both_orders,
    },
    Test {
        name: "ipc_waiters_are_served_in_arrival_order",
        run: ipc_waiters_are_served_in_order,
    },
    Test {
        name: "ipc_call_and_reply_carry_words_and_the_badge",
        run: ipc_call_and_reply,
    },
    Test {
        name: "ipc_waiters_are_woken_with_an_error_when_the_endpoint_goes",
        run: ipc_destroyed_endpoint_wakes_waiters,
    },
    Test {
        name: "ipc_a_caller_is_woken_when_the_server_exits_without_replying",
        run: ipc_caller_woken_when_server_exits,
    },
    Test {
        name: "ipc_reply_without_a_caller_is_refused",
        run: ipc_reply_without_a_caller,
    },
    Test {
        name: "ipc_tasks_talk_through_capabilities_and_a_granted_capability_arrives",
        run: ipc_tasks_with_grant,
    },
    Test {
        name: "ipc_a_capability_cannot_be_sent_without_the_grant_right",
        run: ipc_tasks_without_grant,
    },
    Test {
        name: "ipc_a_task_with_no_endpoint_capability_reaches_nobody",
        run: ipc_stranger_reaches_nobody,
    },
    Test {
        name: "ipc_revoking_an_endpoint_cuts_clients_off_and_wakes_waiters",
        run: ipc_revoke_cuts_off_and_wakes,
    },
    Test {
        name: "join_waits_for_a_thread_and_remembers_the_last_64_that_finished",
        run: join_semantics,
    },
    Test {
        name: "boot_module_numbers_match_the_names_programs_are_started_by",
        run: boot_module_numbers_match,
    },
    Test {
        name: "timer_interrupts_were_counted_during_boot",
        run: timer_interrupts_were_counted,
    },
    #[cfg(feature = "fail-demo")]
    Test {
        name: "deliberate_failure",
        run: deliberate_failure,
    },
];

/// Runs every test, then exits QEMU with success. A failing test never
/// returns: its panic exits QEMU with the failure code.
pub fn run_all() -> ! {
    println!("selftest: running {} tests", TESTS.len());
    for test in TESTS {
        print!("test {} ... ", test.name);
        (test.run)();
        println!("ok");
    }
    println!("selftest: all {} tests passed", TESTS.len());
    qemu::exit(ExitCode::Success)
}
