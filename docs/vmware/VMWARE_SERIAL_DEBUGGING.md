# VMware Serial Debugging

This guide documents the repeatable workflow used to diagnose SunlightOS under
VMware Workstation, including VMXNET3 networking. It keeps the normal virtual
machine configuration recoverable and captures kernel serial output without
depending on the graphical console.

For the VMware SVGA II display driver (PCI `15ad:0405`), see
[`VMWARE_SVGA.md`](VMWARE_SVGA.md). Extract a bounded SVGA trace with:

```bash
rg -n '15ad:0405|\[SVGA\]|display_backend=VMwareSVGA|vmware-svga' \
  /tmp/sunlight-vmware-serial.log
```

## Build the ISO

Use the repository's canonical build path:

```bash
./runs.sh --build
```

The resulting image is:

```text
target/sunlightos.iso
```

The build updates workspace versions as a side effect. If those version changes
are not part of the task, inspect `git status` and revert only the generated
`Cargo.toml`, `Cargo.lock`, and `.version_state.json` changes after testing.

## Locate the VMware VM

The VM used during the VMXNET3 investigation was:

```text
/home/ehsantor/vmware/Rocky Linux 64-bit/Rocky Linux 64-bit.vmx
```

Confirm the important settings before booting:

```bash
rg -n '^(sata0:1|ethernet0|serial0|parallel0|msg\.autoAnswer)' \
  '/home/ehsantor/vmware/Rocky Linux 64-bit/Rocky Linux 64-bit.vmx'
```

For a VMXNET3 NAT test, the configuration should identify:

```text
ethernet0.virtualDev = "vmxnet3"
ethernet0.connectionType = "nat"
ethernet0.present = "TRUE"
```

The CD-ROM should reference the newly built `target/sunlightos.iso`.

## Back Up the VM Configuration

Always preserve the VMX file before adding temporary serial settings:

```bash
VMX='/home/ehsantor/vmware/Rocky Linux 64-bit/Rocky Linux 64-bit.vmx'
cp "$VMX" /tmp/sunlight-vmware-test.vmx.backup
```

Do not edit the virtual disk, generated MAC address, NAT selection, or unrelated
devices for a serial-log test.

## Configure the virtual RTC as UTC

SunlightOS keeps kernel wall time in UTC and applies the configured timezone
once in `timezone_service`. Power the VM off and add this setting to the VMX:

```text
rtc.diffFromUTC = "0"
```

Without it, VMware Workstation can expose host-local civil time through CMOS.
SunlightOS would then interpret that value as UTC and apply the timezone again.
For example, a host-local `+03:30` RTC plus `Asia/Tehran` produces a duplicate
`+03:30` and can advance the displayed date at midnight. The repository's
`tools/runs.sh --vmware` path now refuses to launch a VMX that does not declare
this UTC policy. [Broadcom also documents `rtc.diffFromUTC = 0`](https://knowledge.broadcom.com/external/article/419717/error-bios-time-gets-set-as-local-time-a.html)
as the VMX setting that forces the virtual RTC to UTC.

## Configure a Serial Log

VMware can write COM1 output directly to a host file. Temporarily disable the
parallel port if it is already configured and add:

```text
parallel0.present = "FALSE"
serial0.present = "TRUE"
serial0.fileType = "file"
serial0.fileName = "/tmp/sunlight-vmware-serial.log"
serial0.tryNoRxLoss = "FALSE"
msg.autoAnswer = "TRUE"
```

Use a new or removed log path before each run:

```bash
rm -f /tmp/sunlight-vmware-serial.log
```

Using a fresh path avoids VMware prompts about replacing an existing serial
file. `msg.autoAnswer = "TRUE"` also prevents unattended boots from waiting on
recoverable device prompts.

## Start VMware

For debugging, launching through Workstation proved more reliable than a
detached `vmrun ... nogui` invocation:

```bash
vmware -x "$VMX"
```

`vmrun list` may report zero VMs even while a Workstation-launched test is
starting or shutting down. If state is unclear, check both:

```bash
vmrun list
ps -ef | rg 'vmware-vmx|vmware.*Rocky Linux'
```

Do not start another copy while a stale `*.lck` directory or `vmware-vmx`
process remains.

## Read the Logs

SunlightOS serial output:

```bash
less /tmp/sunlight-vmware-serial.log
```

VMware host/device log:

```bash
less '/home/ehsantor/vmware/Rocky Linux 64-bit/vmware.log'
```

The VMware log is useful for proving that the ISO, NIC, NAT backend, and virtual
cable were attached. The SunlightOS serial log is the source of truth for guest
driver initialization, DMA queues, TX/RX completions, DHCP, and lease state.

Extract a bounded VMXNET3/DHCP trace:

```bash
rg -n \
  '15ad:07b0|\[VMXNET3\]|\[DHCP\]|lease acquired|NETD.*iface|TX desc|TX completion|RX completion|RX frame|smoltcp interface' \
  /tmp/sunlight-vmware-serial.log
```

Inspect VMware attachment evidence:

```bash
rg -n \
  'CDROM: Connecting|Ethernet0 MAC Address|MACVNetConnectToNetwork|link state' \
  '/home/ehsantor/vmware/Rocky Linux 64-bit/vmware.log'
```

## VMXNET3 Verification Sequence

The minimum successful sequence in the serial log is:

1. PCI device `15ad:07b0` is found and its BARs are mapped.
2. VMXNET3 version and UPT version are selected.
3. A valid nonzero unicast MAC address is read.
4. DMA queue structures and rings are initialized.
5. `ACTIVATE_DEV` succeeds and the link query reports the actual state.
6. smoltcp emits a DHCP DISCOVER.
7. VMXNET3 submits the TX descriptor.
8. A genuine TX completion is consumed.
9. A genuine RX completion is consumed and delivered to smoltcp.
10. smoltcp receives OFFER, sends REQUEST, and receives ACK.
11. The lease log includes an address, prefix, gateway, and DNS server.

For the successful July 19, 2026 VMware NAT run, the guest reported:

```text
MAC=00:0c:29:26:2a:9e
lease acquired address=172.16.215.130/24
gateway=172.16.215.2
dns=172.16.215.2
```

Inside SunlightOS, verify:

```text
networkctl status
ping 172.16.215.2
ping 1.1.1.1
```

The successful test returned all four replies from `1.1.1.1`. DNS should then
be tested through the existing resolved path.

## Interpreting Failures

- DISCOVER absent: inspect smoltcp/DHCP startup before the driver.
- DISCOVER logged but no TX submission: inspect the adapter-to-driver TX call.
- TX submitted but no genuine completion: inspect descriptors, DMA visibility,
  notification, and completion progress.
- TX completes but no RX completion: inspect RX posting and VMware network
  configuration.
- RX completes but no smoltcp delivery: inspect length validation and the frame
  proxy.
- OFFER reaches smoltcp but no REQUEST: inspect DHCP socket state handling.
- ACK is received but no address/default route appears: inspect lease
  application and networkd mirroring.

`driver state: Active` and `LINK: carrier` prove neither TX nor RX correctness.

## Restore the VM

After every test, stop the VM and restore the original configuration:

```bash
vmrun stop "$VMX" hard
cp /tmp/sunlight-vmware-test.vmx.backup "$VMX"
```

If `vmrun` says the VM is not powered on, still check for a `vmware-vmx`
process before restoring the file. Finally verify that temporary `serial0`
entries are gone and the original parallel-port setting is back.

## Known Observations

- One test boot showed an intermittent login/session issue; a subsequent boot
  reached the shell and completed networking successfully. Treat that as a
  separate login/session investigation unless serial evidence connects it to
  networking.
- VMXNET3 currently uses one TX queue, one RX queue, and bounded polling rather
  than interrupts. Multiqueue and offloads are intentionally out of scope.
