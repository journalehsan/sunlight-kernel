# SFS-5B.4 models and proof obligations (design before implementation)

A / NORMATIVE: conforming legacy PCI VirtIO block, 1af4:1001, split queue 0,
coherent pinned RAM, whole-device status reset. Accepted 0x240, no RING_RESET.
OASIS VirtIO 1.4 CS01 sections 2.4.1/2.4.2 prohibit queue interaction and
notifications after status-zero completion until reinitialization. Before that,
buffers remain exposed. Pending requests may have completed or been canceled;
the driver cannot infer either result from reset. A compliant device must stop
queue interaction by reset completion, including old exposed request buffers.
Status zero alone is not independent measurement of posted DMA drain, platform
ordering, adversarial bus-master isolation or this emulator's implementation.
It never establishes persistence. Legacy clauses and platform assumptions must remain explicit.
Official reference: https://docs.oasis-open.org/virtio/virtio/v1.4/cs01/virtio-v1.4-cs01.html
The retained 5B.3 normative-fencing-contract.md is prior source-backed analysis,
not a newly executed experiment.

B / INSTRUMENTED_QEMU: isolated QEMU 11.1.2, TCG, ioeventfd=off, no IOThread,
main AioContext and BQL. Timers split real descriptor payload consumption into
bounded steps. Reset can cancel or finish a step in that context. No memory-copy
instruction can execute simultaneously with reset in this model. An operation
pending between callbacks is not actively accessing RAM. Reset-entry copies can
establish O2 but cannot establish O3 or O4 as asynchronous memory overlap. A new
thread reading borrowed mappings without a reviewed lifetime/locking protocol
is rejected as a design, not implemented to force a witness.

C / ADVERSARIAL_NEGATIVE: test-only post-fence payload access before mapping
protection deliberately violates the model fence; observers must detect it.
Its successful detection does not validate the device. Software generations do
not authenticate indistinguishable legacy used-ring heads/status. Do not claim
that perfect same-head forgery is solved.

NATIVE_OBSERVED: actual guest submissions, timeout, reset receipt, CPU/task
execution and pinned inventory; only runtime witnesses qualify these claims.
HOST_MODEL: state-machine/regression assertions, not native races.
UNQUALIFIED: true asynchronous memory instruction/reset overlap, physical DMA,
allocator reuse, platform mapping retirement, hardware durability.

Retirement requires independently qualified acknowledgment, memory-access and
callback quiescence, mapping lifetime, ring/request ownership and exactly-once
accounting. Missing any property keeps all five pages pinned and admission
terminally closed. No new epoch or native SunFS mount is authorized.
