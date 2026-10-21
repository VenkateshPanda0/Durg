# Durg threat model (v0)

> **Status: v0, approved by the owner on 2026-10-15 as the M0 baseline.** Open questions 1 and 2 in section 8 are resolved by adopting the proposals below. Questions 3 to 6 stay open and are decided before the milestone that needs them (see the roadmap).
>
> **Implementation status (after M9):** the kernel runs ring-3 programs loaded from ELF boot modules, each in its own address space, and ends only the faulting task when one misbehaves. That is *evidence* for the supporting property P6 only (see its evidence notes, which also say what they do not show). **None of the five core promises (P1 to P5) is implemented:** there are no capabilities, no labels, no journal, no approval gate and no broker yet. Everything else below is design intent. Nothing here should be read as a security claim until a test or a written argument backs it. This document is the anchor for future claims: if a claim cannot be traced to a section here, it should not be made.

## 1. Purpose and scope

Durg is a capability-based operating system meant to run AI agents and other untrusted automation inside a virtual machine. This model answers four questions for that use case:

1. What are we protecting?
2. Who or what are we protecting it from, and what can they do?
3. Where are the trust boundaries?
4. Which properties must hold, and how will each one be tested?

It is version 0. Scope decisions marked **[open]** need the owner's review before the document is accepted.

## 2. Assets

| ID | Asset | Why it matters |
|---|---|---|
| A1 | **Host system and its data** | The machine running the VMM, its files, credentials and network position |
| A2 | **Secrets** (API keys, tokens, credentials) | Theft is the most common real-world harm from agent compromise |
| A3 | **Guest data the human cares about** | Project files, documents and other objects in the object store |
| A4 | **Integrity of trusted tools** | A trusted tool that executes attacker-influenced input becomes the attack path (the trust handoff) |
| A5 | **The effect journal** | The audit record of what a session did. If it can be forged or silently truncated, the other properties lose their evidence |
| A6 | **Approval authority** | The ability to authorize destructive or external actions. It must stay with the human |
| A7 | **Kernel integrity** | The kernel enforces every other property. Compromising it breaks all of them |
| A8 | **Availability of the guest** | Lower priority, but a session that can wedge the guest denies the human their undo |

## 3. Attacker model

### 3.1 Who the attacker is

The **agent and everything it reads is hostile**. Concretely:

- The agent's code or model output may be adversarial, either because it is malicious or because it has been **prompt-injected** by content it processed (web pages, files, tool output, repository contents).
- Native programs launched inside a session are untrusted.
- Any data entering the guest from outside is untrusted until a trusted component accepts it.

### 3.2 What the attacker can do

| ID | Capability | Notes |
|---|---|---|
| C1 | Execute arbitrary code in user mode inside the guest | Any instruction, any syscall, any argument value |
| C2 | Invoke any capability it holds, in any order, at any rate | Including malformed invocations |
| C3 | Create, modify and name objects within the capabilities it holds | Including files that trusted tools might later consume |
| C4 | Send arbitrary messages to any endpoint it holds a capability for | Includes crafted inputs aimed at services |
| C5 | Spawn many threads and tasks, and consume resources it is given | Resource exhaustion within its own budget |
| C6 | Produce text designed to manipulate a human reading the console | Social engineering of the approval step |
| C7 | Observe anything it has a capability to observe | Including timing it can measure from user mode |

### 3.3 What the attacker cannot do (assumptions)

These are assumptions the model depends on. If one fails, the corresponding guarantees fail.

| ID | Assumption |
|---|---|
| S1 | The **host, VMM and hypervisor are trusted and uncompromised** |
| S2 | The **hardware behaves as specified** (no hardware attacks, no CPU bugs exploited) |
| S3 | The **boot chain** (firmware and Limine bootloader) is trusted |
| S4 | The **human console is operated by a person** who is not the attacker |
| S5 | The **build toolchain** and the Durg source are not malicious |
| S6 | The agent **cannot send input through the human console** except as displayed output |

## 4. Scope

### 4.1 In scope

- Code in the guest escaping its capabilities (confused deputy, forged or leaked capability, kernel bug reachable from user mode)
- Reading or modifying objects the session was not given a capability for
- **Trust-handoff attacks**: an agent-created artifact later executed by a more trusted principal without acceptance
- Forging, suppressing or rewriting entries in the effect journal from inside the guest
- Obtaining approval authority, or tricking the approval path into minting it
- Obtaining secret values rather than broker handles
- Network egress that bypasses the broker
- A malicious or buggy **user-space driver or service** compromising other components or the kernel
- Kernel memory exhaustion by a user task

### 4.2 Out of scope (v0)

| Threat | Reason |
|---|---|
| **Malicious host, VMM or hypervisor** | Host trust is assumption S1. Durg protects the host *from* the guest, not the reverse |
| **Hardware and physical attacks** | Assumption S2 |
| **Timing and microarchitectural side channels** (Spectre-class, cache, covert channels) | Mitigations are a research effort of their own. Not claimed |
| **Denial of service by the host** | The host controls the VM |
| **Compromise of the human or the console** | Assumption S4 |
| **Supply-chain compromise** of toolchain or dependencies | Assumption S5. Partly mitigated by the dependency policy, but not claimed |
| **Persuading the human to approve a harmful action** with accurate information | The human is the authority. Design should make requests clear, but we do not claim to stop an informed approval |
| **Vulnerabilities in the agent's own logic or the model** | Not an OS concern. Durg limits the blast radius, it does not fix the agent |
| **Confidentiality of data against the agent for objects it was legitimately given** | A capability grants access. Durg does not prevent misuse of what was granted |

**[resolved]** Availability against a hostile session (A8) is in scope only for guarantees that affect the human's ability to undo, plus kernel memory exhaustion.

## 5. Trust boundaries

```
 ┌────────────────────────── HOST (trusted, assumption S1) ─────────────────────┐
 │  Agent loop + LLM    Controller    Egress broker    Human console             │
 └───────────────────────────────┬───────────────────────────────────────────────┘
                                 │  B1: vsock (host ↔ guest)
 ┌───────────────────────────────┴──── GUEST ────────────────────────────────────┐
 │  ┌─────────────────────── Kernel (ring 0) ───────────────────────────────┐   │
 │  │ capabilities · IPC · memory · threads · traps                            │   │
 │  └──────────────────────────────┬───────────────────────────────────────────┘   │
 │                                 │  B2: syscall boundary (ring 3 → ring 0)       │
 │  ┌── Trusted services ──┐   B3: IPC between services  ┌── Untrusted session ──┐ │
 │  │ object store, journal │◄────────────────────────────►│ agent + its tools     │ │
 │  │ label svc, approval   │   (only via held endpoints)  │ (hostile, §3)         │ │
 │  │ virtio drivers        │                              └───────────────────────┘ │
 │  └───────────────────────┘                                                         │
 └────────────────────────────────────────────────────────────────────────────────────┘
```

| ID | Boundary | What crosses it | Main risk |
|---|---|---|---|
| B1 | Host ↔ guest (vsock) | Tool calls in, results out, broker requests, approval minting | Malformed host input, broker policy bypass, forged approval |
| B2 | User mode ↔ kernel (syscalls) | Capability invocations | Kernel memory corruption, capability forgery, confused deputy |
| B3 | Between user-space services and the session | IPC messages over endpoints | Services trusting message contents, label stripping |
| B4 | Driver ↔ device (virtio) | Descriptors and data in shared memory | A malicious host-supplied device state corrupting the driver |
| B5 | Trusted tool ↔ agent-created objects | Files, labeled objects | The trust handoff |

**[resolved]** B4 treats the host as trusted (S1) but still validates virtio input defensively, since driver robustness is cheap insurance.

## 6. Core promises as testable properties

Each promise is restated as a property with an intended test. **None of these hold yet.** "Test sketch" describes how a future test could falsify the property.

### P1. No ambient authority
**Property.** A task can only perform an operation on an object if it holds a capability that authorizes that operation on that object. There is no global namespace, and no process holds authority it was not explicitly given.
**Defends against:** C1, C2, C3 (escape, enumeration of the host or other sessions).
**Test sketch.**
- A user task with an empty capability space attempts every syscall with every argument pattern and gains no effect beyond its own task.
- A task cannot name, open or enumerate any object it was not handed.
- Fuzz the syscall interface with random capability slots and arguments. Assert no access to unheld objects and no kernel fault.
- Property tests on derivation and revocation: after revocation, no derived capability remains usable.

### P2. Provenance labels
**Property.** Every object created or modified in a session carries that session's label. A more-trusted principal cannot execute or consume a labeled object without an explicit, journaled acceptance.
**Defends against:** the trust handoff (B5), C3.
**Test sketch.**
- The scripted demo: an agent writes a git-hook-style file, the trusted tool refuses to run it, then runs it after acceptance and the acceptance appears in the journal.
- Label preservation: copy, move, derive and rewrite operations on a labeled object never produce an unlabeled result.
- A session cannot remove or alter a label on its own objects.
**Known gap.** Labels are enforced by services, not the kernel. This keeps the kernel small but means label correctness depends on the store and loader being correct and on every consuming path going through them. **[open]** Label semantics beyond "one label per session plus an accept operation" are undecided.

### P3. Effect journal and session undo
**Property.** Every capability-mediated side effect is appended to a hash-chained log the session cannot modify, and a whole session can be rolled back to its starting snapshot.
**Defends against:** A5, and gives recovery from any in-scope compromise that stays within session-held capabilities.
**Test sketch.**
- Tamper test: modifying or removing an entry breaks chain verification.
- Crash-consistency test: kill the guest at random points and check that the journal and store recover to a consistent state.
- Undo test: after a scripted hostile session, the store is byte-identical to the pre-session snapshot, except for the journal itself.
**Known limits.** A hash chain inside the guest proves internal consistency, not completeness against someone who can rewrite the whole chain. Anchoring checkpoints on the host is an option. **[open]** Journal tamper-evidence scheme.

### P4. Approval as a capability
**Property.** Destructive or external operations require a one-time approval capability. Only the human-controlled console path can mint one. No agent action and no guest-side prompt can create one.
**Defends against:** A6, C6.
**Test sketch.**
- No code path in the guest can create an approval capability. Enumerate minting entry points and assert there is exactly one, on the host-originated channel.
- An approval capability is consumed on first use, and replay fails.
- A scripted session attempts the gated operation without approval and is refused, with the refusal journaled.
**Known limit.** The agent can still try to persuade the human (C6). Clear, accurate presentation of what is being approved is a design requirement, but it is not a guarantee. See §4.2.

### P5. Brokered egress and secrets
**Property.** The guest has no raw network access. All egress goes through the host-side broker, which applies policy and injects credentials. The agent only ever sees handles, never secret values.
**Defends against:** A1, A2.
**Test sketch.**
- No network device or socket capability exists in the guest in phase 1. Assert it by enumerating devices and capabilities.
- A session that tries to read a secret receives only a handle, and the handle is useless outside the broker.
- A request violating broker policy is refused and journaled.
- Search guest memory, store and journal for a known planted secret value after a session. Assert it is absent.

### P6. Kernel integrity (supporting property)

> **Evidence so far (M5), not a claim that the property holds:** the kernel image is mapped with W^X (no page writable and executable, checked page by page); writing read-only data and executing data both fault; the image has no writable alias in the direct map; the kernel stack has an unmapped guard page and an overflow ends in a double fault on a separate stack, traced to the guard page.

> **Evidence so far (M8), not a claim that the property holds.** A program now runs in ring 3 in its own address space, and each of these was shown, in a QEMU test, to end *only that task* while the kernel carried on to the next test, with the exact exception and page-fault error code checked: reading kernel memory, writing kernel memory, jumping into kernel memory (the fault must be at the first instruction), reading an unmapped page, writing to its own read-execute code page, running code from its own stack (also checked at the first instruction), and the privileged instructions `cli`, `hlt`, `mov cr3` and port I/O. Bad `debug_print` arguments (a kernel pointer, an unmapped pointer, an over-long length, a wrapping range, an unknown call number) are refused and the task continues. The kernel never dereferences a user pointer: it validates the range, translates each page in the caller's tables, requires the page to be user-accessible, and copies through its own mapping. The pure validator has property tests and 8 of 8 mutations caught, plus two libFuzzer targets (run for 30 to 60 seconds each by hand, tens of millions of executions, no failure on the correct code, and all 7 validator mutations tried were found within seconds), and 22,000 random and boundary calls into the real handler from inside a user address space produced no wrong result. Program output has control characters replaced. A new task starts with every register zeroed (a kernel function pointer was in one, and a mutation that left it showed the test catches it). SMEP, SMAP and UMIP are enabled when the CPU reports them and each is tested to refuse what it should, **under QEMU TCG with those features added to the CPU model; hardware and KVM are untested.** Most of these tests were checked by breaking the matching mechanism on purpose (user-code and stack permissions, the user-accessible check in the copy path, console sanitising, register zeroing, the kernel-stack switches, ring-3 fault handling, kill, resource freeing, the system-call flag mask, SMEP, SMAP and UMIP). The privileged-instruction cases (`cli`, `hlt`, `mov cr3`, port I/O) and the reads, writes and jumps into kernel memory rest on what the hardware does with the GDT and page-table bits, and no mutation was made for them.
>
> **What M8 does not show.** One CPU only. The `sysret` non-canonical-address defence and the exception-delivery AC-flag behaviour are written but not exercised by any test. A non-maskable interrupt arriving in the few instructions between `syscall` and the stack switch is not handled. Task kill takes effect only when a tick finds the task in ring 3. Coverage-guided fuzzing (libFuzzer, ADR-0008) covers only the pure decoding, validation and splitting logic, not the kernel's page-table walk, the copy through the direct map, or the entry stub, and it is run by hand, not in CI. The call table has three calls and is not the capability ABI, so this says little about T2. A task cannot make the kernel allocate (the kernel allocates at spawn), so T9 is untested rather than demonstrated.

> **Evidence so far (M9), not a claim that the property holds.** Separately built programs are now loaded from boot modules by an ELF parser that fails closed (it rejects writable-and-executable segments, overlapping or out-of-range segments, dynamic, interpreter and TLS segments, an executable stack and an entry point outside executable file bytes; 22 host tests including property tests, plus a libFuzzer target). In QEMU, every page of a loaded program has exactly the permissions of its segment (checked on the page-table path at every level, with a property test over every boot module). Two programs built to keep a secret at the **same virtual address**, run interleaved, each kept its own secret and never saw the other's, and the two address spaces own disjoint physical frames. A frame that held another task's data reads as zeros in the next task (a task that fills its memory with junk, then a probe that counts non-zero bytes in frames all known to have held junk). Boot-module memory is mapped read-only, no-execute and kernel-only. Of 8 deliberate breakages of the loader tried (shared first data page, no frame zeroing, non-executable segments writable, no-execute never set, pages not user-accessible, module window writable, module window executable, file bytes ignored), 7 were caught by the tests as first written; the 8th (no frame zeroing) **survived** and exposed a weak test, which was fixed and then caught it.
>
> **What M9 does not show.** The shared-page mutation was caught first by the structural frame-disjointness assertion, so I did not separately prove that the secret-keeping programs alone would catch it. The direct map still gives the kernel a writable alias of every user page. Programs are not relocated and hold no capabilities, so this says nothing about T2. The fuzz target covers the parser, not the loader's page-building, and is not in CI. Everything ran under QEMU TCG.

**Property.** A user-mode task cannot corrupt kernel state or crash the kernel. A faulting user task is killed, and the kernel survives.
**Defends against:** A7, kernel memory exhaustion.
**Test sketch.**
- A user task that touches kernel memory is killed (QEMU test).
- The kernel does not allocate on behalf of user tasks after boot. Tasks retype untyped memory they hold.
- Fuzz the syscall entry. Track the `unsafe` line count as a metric.
**Known limit.** Not formally verified. The kernel is expected to contain bugs, and this property is a target, not a proof.

## 7. Threat catalogue

Threats mapped to the properties meant to address them. "Coverage" is *designed* coverage, not demonstrated.

| # | Threat | Boundary | Addressed by | Coverage |
|---|---|---|---|---|
| T1 | Agent writes a file that a trusted tool executes (git hook, editor task, interpreter config) | B5 | P2 | designed |
| T2 | Agent forges or guesses a capability | B2 | P1, P6 | designed |
| T3 | Confused deputy: a service performs an action using its own authority on the agent's behalf | B3 | P1 | **gap, needs design rules for services** |
| T4 | Agent modifies or truncates the journal | B3 | P3 | designed (limits noted) |
| T5 | Agent mints or replays an approval | B1 | P4 | designed |
| T6 | Agent reads a secret | B1, B3 | P5 | designed |
| T7 | Agent opens raw network egress | B1 | P5 | designed |
| T8 | Syscall input corrupts the kernel | B2 | P6 | designed; partly exercised in M8 (see P6 evidence) |
| T9 | Agent exhausts kernel memory | B2 | P6 (no kernel allocation for tasks) | designed |
| T10 | Malicious virtio device state corrupts a driver | B4 | defensive validation (section 8, question 2) | designed, no driver exists yet |
| T11 | Agent crafts messages that exploit a service parser | B3 | service-level fuzzing, minimal parsers | **gap** |
| T12 | Agent strips or launders labels via an unlabeled path | B3, B5 | P2 | **depends on label semantics** |
| T13 | Agent manipulates the human into approving | B1 | P4 plus clear UI | **out of scope to guarantee** |
| T14 | Side channels between sessions | n/a | none | **out of scope** |

## 8. Open questions for the owner

1. ~~**Availability scope.**~~ Resolved: in scope only where it affects undo, plus kernel memory exhaustion.
2. ~~**Virtio stance.**~~ Resolved: drivers validate device input defensively.
3. **Label semantics** (decide before M13). One label per session plus a single accept operation, or something richer?
4. **Journal anchoring** (decide before M13). Is a hash chain inside the guest sufficient for v0, or should checkpoints be anchored host-side?
5. **Confused-deputy rules** (decide before M10). Which design rules do trusted services follow to avoid acting on the agent's behalf with their own authority?

   **Proposed rules, not decided (assistant's draft, for the owner to accept or change):**
   1. **Designation comes with authority.** A service acts on an object only through a capability the *request carried*, never through one it happens to hold for its own purposes. If a request names something, it must hand over (or badge-identify) the right to it.
   2. **No ambient lookup by name.** Services never resolve a client-supplied name or path using their own wider authority; there is no global namespace to resolve in.
   3. **Identify clients by badge, not by claim.** A service learns who is calling from the badge on the endpoint capability it was given, never from a field the client writes.
   4. **A service holds the least it needs and no more,** and a capability it must keep for itself is never placed in a reply or a shared region.
   5. **Revocation reaches in-flight work.** When a session's capabilities are revoked, services drop the work they were doing on its behalf.
   These are design rules to follow when the root task and services are written; none is enforced or tested yet.
6. **Multiple concurrent sessions** (decide before M13). Is v0 single-session, or must isolation between concurrent sessions be addressed now?

## 9. How this document is used

- Every security statement in the README, docs or code must trace to a property or assumption here.
- When a property gets a passing automated test, record that test next to the property. Until then, it stays "design intent".
- Changes to scope or assumptions are made through an ADR, then reflected here.
- This is version 0. It is expected to change as the design firms up, and as outside reviewers find holes. That feedback is welcome in the issue tracker.
