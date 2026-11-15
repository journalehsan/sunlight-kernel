from pathlib import Path
import tarfile,subprocess
R=Path('/home/ehsantor/Projects/sunlightos-kernel'); O=R/'docs/audit/sunfs/sfs-5b4'
S=Path('/tmp/sfs5b4-qemu-src');S.mkdir(exist_ok=True)
with tarfile.open('/tmp/sfs5b3-qemu-11.1.2.tar.xz') as t:
 for m in t.getmembers():
  m.name=m.name.split('/',1)[-1]
  if m.name and not (m.issym() and m.linkname.startswith('/')): t.extract(m,S,filter='data')
subprocess.run(['patch','-d',str(S),'-p1','-i',str(R/'docs/audit/sunfs/sfs-5b3/qemu-active-io.patch')],check=True)
p=S/'hw/block/virtio-blk.c';s=p.read_text()
s=s.replace('static struct iovec sfs5b3_iov;','''static struct iovec sfs5b3_iov;
static uint64_t sfs5b4_seq;
static unsigned sfs5b4_budget;
static unsigned sfs5b4_head;
static bool sfs5b4_fenced;
static QEMUTimer *sfs5b4_late;
static void sfs5b4_event(const char *stage, unsigned bytes)
{
    assert(bql_locked());
    fprintf(stderr, "SFS5B4 seq=%" PRIu64 " ns=%" PRId64 " epoch=1 queue=0 head=%u stage=%s bytes=%u\\n",
            ++sfs5b4_seq, qemu_clock_get_ns(QEMU_CLOCK_REALTIME),
            sfs5b4_head, stage, bytes);
}
static void sfs5b4_late_observer(void *opaque)
{
    /* Scalar-only observer. No request or guest pointer survives teardown. */
    assert(sfs5b4_fenced);
    sfs5b4_event("LATE_CALLBACK_NO_REQUEST", 0);
    sfs5b4_event("STALE_ACCESS_REJECTED", 0);
}
''')
s=s.replace('    if (!release || access(release, F_OK)) {','''    if (!release || access(release, F_OK)) {
        if (++sfs5b4_budget == 3000) {
            sfs5b4_event("BARRIER_EXPIRED_RETAINED", 0);
            return; /* held until reset or process termination; no rearming */
        }''')
s=s.replace('    assert(qemu_iovec_to_buf(&sfs5b3_guest, 256,', '    sfs5b4_event("PAYLOAD_ACCESS_STARTED", 0);\n    assert(qemu_iovec_to_buf(&sfs5b3_guest, 256,',1)
s=s.replace('    sfs5b3_held = NULL;\n    sfs5b3_iov', '    sfs5b4_event("PAYLOAD_ACCESS_ENDED", 256);\n    sfs5b3_held = NULL;\n    sfs5b3_iov',1)
s=s.replace('    assert(pipe2(fds, O_NONBLOCK | O_CLOEXEC) == 0);','''    sfs5b4_fenced = true;
    sfs5b4_event("RESET_ACKNOWLEDGED", 0);
    if (!strcmp(getenv("SFS5B3_MODE"), "negative")) {
        /* Deliberately broken TEST model: original stable RAM still pinned.
         * Kernel pipe-copy safely witnesses a real illegal post-fence read.
         * No physical DMA or hostile alias-containment claim. */
        assert(pipe2(fds, O_NONBLOCK | O_CLOEXEC) == 0);
        assert(write(fds[1], sfs5b3_old_data, 1) == 1);
        close(fds[0]); close(fds[1]);
        sfs5b4_event("ILLEGAL_POST_FENCE_ACCESS", 1);
    }
    if (!sfs5b4_late) {
        sfs5b4_late = timer_new_ms(QEMU_CLOCK_REALTIME, sfs5b4_late_observer, NULL);
    }
    timer_mod(sfs5b4_late, qemu_clock_get_ms(QEMU_CLOCK_REALTIME) + 20);
    assert(pipe2(fds, O_NONBLOCK | O_CLOEXEC) == 0);''')
s=s.replace('    sfs5b3_old_data = NULL;','    sfs5b4_event("ORIGINAL_MAPPINGS_PROTECTED", 0);\n    sfs5b3_old_data = NULL;',1)
s=s.replace('    sfs5b3_guest = req->qiov;','''    sfs5b4_head = req->elem.index;
    sfs5b4_fenced = false;
    sfs5b4_budget = 0;
    sfs5b4_event("DEVICE_FETCHED", 0);
    sfs5b3_guest = req->qiov;
    sfs5b4_event("PAYLOAD_ACCESS_STARTED", 0);''')
s=s.replace('    sfs5b3_held = req;','    sfs5b4_event("PAYLOAD_ACCESS_ENDED", 256);\n    sfs5b3_held = req;\n    sfs5b4_event("REQUEST_OUTSTANDING", 0);',1)
s=s.replace('        sfs5b3_old_data = sfs5b3_guest.iov[0].iov_base;','''        sfs5b4_event("RESET_ENTERED", 0);
        const char *mode = getenv("SFS5B3_MODE");
        if (!strcmp(mode, "reset-read")) {
            sfs5b4_event("PAYLOAD_ACCESS_STARTED", 0);
            assert(qemu_iovec_to_buf(&sfs5b3_guest, 256,
                                    sfs5b3_data + 256, 256) == 256);
            sfs5b4_event("PAYLOAD_ACCESS_ENDED", 256);
        }
        if (!strcmp(mode, "error")) {
            sfs5b4_event("DEVICE_ERROR_CANCEL", 0);
        }
        sfs5b3_old_data = sfs5b3_guest.iov[0].iov_base;''')
s=s.replace('        g_free(req);\n    }\n\n    /* Dataplane', '        g_free(req);\n        sfs5b4_event("OLD_EPOCH_QUIESCED", 0);\n    }\n\n    /* Dataplane',1)
p.write_text(s)
# Full patch against archive originals, not an incremental patch on prior source.
import tempfile
with tempfile.TemporaryDirectory(prefix='sfs5b4-orig-') as tmp:
 with tarfile.open('/tmp/sfs5b3-qemu-11.1.2.tar.xz') as t:
  patch=''
  for n in ['hw/block/virtio-blk.c','hw/virtio/virtio.c']:
   orig=t.extractfile('qemu-11.1.2/'+n).read(); f=Path(tmp)/Path(n).name;f.write_bytes(orig)
   r=subprocess.run(['diff','-u','--label','a/'+n,'--label','b/'+n,str(f),str(S/n)],capture_output=True)
   patch+=r.stdout.decode()
 (O/'qemu-async-access.patch').write_text(patch)
print(S)
