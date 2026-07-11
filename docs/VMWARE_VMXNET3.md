# VMware VMXNET3 networking

SunlightOS selects VMXNET3 only after PCI probing finds VMware device
`15ad:07b0` and the device completes reset, revision/UPT negotiation, queue
setup, activation, and RX-mode configuration. It does not infer the backend
from the hypervisor or build target.

Fully power off the VM (do not merely suspend it) before changing
`ethernet0.virtualDev`. The following
`.vmx` settings configure a connected VMXNET3 adapter on VMware NAT:

```text
ethernet0.present = "TRUE"
ethernet0.connectionType = "nat"
ethernet0.virtualDev = "vmxnet3"
ethernet0.startConnected = "TRUE"
```

On the next boot, check all four settings in the powered-off VM's `.vmx` file,
then trust the enumerated PCI ID rather than the fact that VMware is the
hypervisor. Serial output must contain
`[VMXNET3] pci device 15ad:07b0 found at ...`, each staged transition through
`DeviceActivated`, `[VMXNET3] ACTIVATE_DEV succeeded`,
`[NET] active backend query returned VMXNET3`, and the generic frame-backend
registration.
`networkctl list` must show `VMXNET3` in the Kind column. If the PCI ID is not
present, SunlightOS prints a configuration warning and may select an actually
detected VirtIO-Net device instead; it never creates a synthetic VMXNET3
backend.

Interface publication is independent of DHCP and carrier. The kernel keeps the
selected backend and its DMA rings in the global active-backend slot, net_server
registers that backend with deviced before attempting DHCP, and networkd force
refreshes discovery when the registration notification arrives. A connected
device with link down still publishes `eth0` with `NoCarrier`.

Before investigating DHCP, verify:

```text
networkctl refresh
networkctl list
networkctl status
networkctl status eth0
networkctl down eth0
networkctl up eth0
networkctl list
```

`eth0` must remain present while administratively disabled, Kind must remain
`VMXNET3`, and Link must report carrier separately from Admin state.

If the adapter is missing, serial output includes:

```text
[VMXNET3] pci device 15ad:07b0 not present
[VMXNET3] verify ethernet0.virtualDev = "vmxnet3"
```

If the PCI ID exists but initialization fails, serial output names the exact
stage, including BAR decode, DMA allocation, reset, revision negotiation, UPT
negotiation, MAC acquisition, activation, or RX-mode update.

The initial driver uses bounded polling from the network-service cadence.
Completion handling is not tied to the GUI and does not busy-spin. During DHCP,
the first TX/RX descriptors and rate-limited Layer-2 counters identify whether
progress stopped before TX submission, at TX completion, at RX completion, or
at delivery into smoltcp.
