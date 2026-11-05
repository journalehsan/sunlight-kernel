//! Explicit hybrid fixture: one native request ring and a real IPC reply.
use super::*;
use sunlight_ipc::fastlane_operation::{Frame, Operation, Transport};
pub mod lifecycle;
const SETUP: u64 = 0x6e01;
const EXECUTE: u64 = 0x6e02;
const FORWARD: u64 = 0x6e03;
const REPLY_PROBE: u64 = 0x6e04;
pub struct Endpoint {
    request: Option<ring::Consumer<Native>>,
    handles: Option<fl::Handle>,
    operation: Option<Operation>,
    last_reply: Option<u64>,
    held_response: Option<Frame>,
    consumer_page: u64,
}
fn memory(h: fl::Handle, role: u64) -> Native {
    let mut pages = [0; 3];
    for r in 0..3 {
        let (va, size) = fl::call(8, h, role * 3 + r, fl::NO_SESSION).unwrap();
        assert_eq!(size, 4096);
        assert_eq!(va % 4096, 0);
        pages[r as usize] = va;
    }
    fl::operate(24, h, role).unwrap();
    Native { h, parent: CapabilityToken(0), pages, role }
}
fn receive(c: &mut ring::Consumer<Native>, h: fl::Handle) -> Frame {
    // Publication is completed before this authenticated control call. Observe
    // the existing M4 sequence; the notification itself carries no payload.
    assert!(fl::operate(11, h, 0).unwrap() > 0);
    let lease = c.try_read().unwrap();
    assert_eq!(lease.identity().channel, h.generation);
    assert_eq!(lease.payload().len(), 32);
    let frame = Frame::from_ring(lease.payload()).unwrap();
    lease.release().unwrap();
    frame
}
impl Endpoint {
    pub fn new() -> Self { Self { request: None, handles: None, operation: None, last_reply: None, held_response: None, consumer_page: 0 } }
    pub fn handle(&mut self, m: IpcMsg) -> bool {
        if m.label == lifecycle::caller_exec::RELEASE {
            lifecycle::caller_exec::release(self, m);
            return true;
        }
        if m.label == FORWARD {
            assert_eq!(m.cap_count, 1);
            let response = sunlight_ipc::ipc_call_timeout(m.caps[0],
                IpcMsg::with_label(CMD).word(0,35).word(1,m.words[0]).word(2,m.words[1]).word(3,0x10010),5000).unwrap();
            sunlight_ipc::ipc_reply_result(response).unwrap();
            return true;
        }
        if m.label == REPLY_PROBE {
            // An invented target must not consume this call's ordinary target.
            assert!(sunlight_ipc::ipc_complete_deferred_reply(u64::MAX,IpcMsg::with_label(0)).is_err());
            let old = self.last_reply.unwrap();
            assert!(sunlight_ipc::ipc_complete_deferred_reply(old,IpcMsg::with_label(0)).is_err());
            sunlight_ipc::ipc_reply_result(IpcMsg::with_label(REPLY_PROBE).word(0,old)).unwrap();
            return true;
        }
        if m.label == SETUP {
            assert!(self.handles.is_none());
            let h = fl::Handle { token: m.words[0], generation: m.words[1] };
            // M1 authenticates each role; the later operation check also binds
            // the IPC caller to the committed request channel's creator.
            let native = memory(h, 1);
            self.consumer_page = native.pages[1];
            self.request = Some(ring::Consumer::new(native, h.generation).unwrap());
            self.handles = Some(h);
            sunlight_ipc::ipc_reply_result(IpcMsg::with_label(SETUP)).unwrap();
            return true;
        }
        if m.label == lifecycle::EXECUTE_R2 {
            lifecycle::execute(self,m);
            return true;
        }
        if m.label != EXECUTE { return false; }
        let h = self.handles.unwrap();
        assert_eq!(m.word_count, 4);
        assert_eq!(m.cap_count, 0);
        assert_eq!(m.words, [h.token, h.generation, 0x10010, 1, 0, 0, 0, 0]);
        assert_eq!(fl::operate(35,h,0x10010),Ok(2));
        assert!(self.operation.is_none());
        let reply_token = sunlight_ipc::ipc_defer_reply().unwrap();
        assert!(sunlight_ipc::ipc_deferred_reply_is_live(reply_token));
        sunlight_ipc::debug_log(&alloc::format!("[FL-M61E] case=reply-created channel={} token={} request_id=1 operation=65552", h.generation, reply_token));
        let request = receive(self.request.as_mut().unwrap(),h);
        let mut operation = Operation::committed(Transport::Ring);
        assert_eq!(request.request_id(), m.words[3]);
        let response = operation.execute(request).unwrap();
        sunlight_ipc::debug_log(&alloc::format!("[FL-M61E] case=private-copy channel={} actor={} authorized=1 bytes=32",h.generation,sunlight_libc::getpid()));
        sunlight_ipc::debug_log("[FL-M61E] case=actor-executed operation=65552 request=ring reply=ipc");
        self.operation = Some(operation);
        self.last_reply = Some(reply_token);
        assert!(fl::operate(5,h,0).is_ok());
        sunlight_ipc::ipc_complete_deferred_reply(reply_token, response.ipc_message()).unwrap();
        assert!(!sunlight_ipc::ipc_deferred_reply_is_live(reply_token));
        assert!(sunlight_ipc::ipc_complete_deferred_reply(reply_token, response.ipc_message()).is_err());
        sunlight_ipc::debug_log("[FL-M61E] case=duplicate-reply terminal=rejected count=1");
        sunlight_ipc::debug_log(&alloc::format!("[FL-M61E] case=response-published channel={} transport=ipc request_id=1 bytes=32",h.generation));
        true
    }
}
pub fn supervisor() -> ! {
    doorbell::pin(0);
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    let parent = alloc::format!("{}",public.0);
    let pid = sunlight_libc::spawn(b"/test/contract-required", &[b"session-racer",b"FL-actor",parent.as_bytes()],None).unwrap();
    let hello = sunlight_ipc::ipc_recv_timeout(ep,5000).unwrap();
    assert_eq!(hello.label,HELLO); assert_eq!(hello.badge,pid);
    let actor = Actor { pid, send: hello.caps[0] };
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(HELLO)).unwrap();
    let h = offer();
    let consent = expect(actor,AUTHORIZE,h,1,0);
    assert_eq!(fl::operate(31,h,0x10202),Ok(2));
    fl::operate(2,h,consent.words[1]).unwrap();
    fl::operate(3,h,0).unwrap(); expect(actor,3,h,1,0);
    let mut producer = ring::Producer::new(memory(h,0),h.generation).unwrap();
    let setup = IpcMsg::with_label(SETUP).word(0,h.token).word(1,h.generation);
    assert_eq!(sunlight_ipc::ipc_call_timeout(actor.send,setup,5000).unwrap().label,SETUP);
    assert_eq!(fl::operate(32,h,0),Ok(2));
    sunlight_ipc::debug_log("[FL-M61E] case=directional operation=65552 request=FastLaneRequired reply=KernelIPC fallback=0");
    let request = Frame::request(1,b"0123456789abcdef").unwrap();
    producer.try_send(&request.ring_bytes()).unwrap();
    fl::operate(10,h,0).unwrap();
    sunlight_ipc::debug_log(&alloc::format!("[FL-M61E] case=request-published channel={} caller={} bytes=32",h.generation,sunlight_libc::getpid()));
    let control = IpcMsg::with_label(EXECUTE).word(0,h.token).word(1,h.generation).word(2,0x10010).word(3,1);
    let response = sunlight_ipc::ipc_call_timeout(actor.send,control,5000).unwrap();
    assert_eq!(Frame::from_ipc_message(&response),Ok(request.transform().unwrap()));
    sunlight_ipc::debug_log("[FL-M61E] case=acknowledged request=0123456789abcdef response=fedcba9876543210 terminal=success");
    // Essential bounded probes run only after the positive operation accepted.
    expect(actor,35,h,0x10011,1);
    sunlight_ipc::debug_log("[FL-M61E] case=negative control=unauthorized-operation terminal=rejected count=1");
    expect(actor,35,fl::Handle { generation: h.generation + 1, ..h },0x10010,2);
    sunlight_ipc::debug_log("[FL-M61E] case=negative control=stale-channel terminal=rejected count=1");
    let foreign = spawn(ep,public);
    let forwarded = IpcMsg::with_label(FORWARD).word(0,h.token).word(1,h.generation).with_cap(0,actor.send);
    let denied = sunlight_ipc::ipc_call_timeout(foreign.send,forwarded,5000).unwrap();
    assert_eq!(denied.words[0],1);
    sunlight_ipc::debug_log("[FL-M61E] case=negative control=foreign-caller terminal=rejected count=1");
    stop(foreign);
    assert_eq!(authorize(0,fl::EMPTY,30000),Err(3));
    expect(actor,AUTHORIZE,fl::EMPTY,60000,3);
    sunlight_ipc::debug_log("[FL-M61E] case=negative control=second-ring terminal=rejected count=1");
    let probe = sunlight_ipc::ipc_call_timeout(actor.send,IpcMsg::with_label(REPLY_PROBE),5000).unwrap();
    assert_eq!(probe.label,REPLY_PROBE);
    assert!(sunlight_ipc::ipc_complete_deferred_reply(probe.words[0],IpcMsg::with_label(0)).is_err());
    sunlight_ipc::debug_log("[FL-M61E] case=negative control=forged-reply-target terminal=rejected count=1");
    sunlight_ipc::debug_log("[FL-M61E] case=negative control=late-reply terminal=rejected count=1");
    fl::operate(4,h,0).unwrap();
    stop(actor); mapping::census(690); zero();
    sunlight_ipc::debug_log(&alloc::format!("[FL-M61E] case=complete pid={} request_channel={} request=ring reply=ipc",sunlight_libc::getpid(),h.generation));
    sunlight_libc::exit(0)
}
