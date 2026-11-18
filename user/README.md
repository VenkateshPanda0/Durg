# `user/`: ring-3 programs (`durg-user`)

`src/lib.rs` is the small runtime every user program links: the system-call
stubs (`print`, `yield_now`, `exit`, `invoke`, `spawn`, `join`, `send`, `recv`,
`call`, `reply`, `notification_signal`, `notification_wait`) and a panic
handler. `user.ld` places each program's code at `0x400000` and its data at
`0x600000`. Each program in `src/bin/` becomes one ELF file that the bootloader
loads as a boot module; the module order in `../boot/limine.conf` is the
numbering `spawn` uses (`durg_syscall::module`).

| Program | Role |
|---|---|
| `init` | **The root task.** Starts with its own capability space, 4 MiB of untyped memory and the spawner; builds and checks the demo system at every boot |
| `ipc_server`, `ipc_client`, `ipc_stranger` | The message-passing scenario: a receive-only server, a badged send-only client, and a task given nothing that must reach nobody |
| `capdemo` | Exercises capability operations from ring 3; its exit code is the first failing step |
| `task_a`, `task_b` | The isolation test: each keeps a secret at the same address, neither may see the other's |
| `dirty`, `probe` | Check that memory a task leaves behind reaches the next task zeroed |
| `crasher` | Faults on purpose, to check that `join` reports an exception |
| `notification_waiter` | Notification delivery and cancellation tests |

To add a program: create `src/bin/NAME.rs` and a `[[bin]]` entry in `Cargo.toml`,
add it to `USER_PROGRAMS` in `../xtask/src/main.rs`, to `../boot/limine.conf`, to
`PROGRAM_NAMES` in `../kernel/src/user.rs`, to `MODULES` in
`../kernel/src/selftest.rs` and to `durg_syscall::module` (all in the same
order); the self-test `boot_module_numbers_match` checks that they agree.
