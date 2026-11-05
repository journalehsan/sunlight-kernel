use super::*;
use sunlight_ipc::fastlane_operation::{Frame, Operation, Transport};
const BIND: u64 = 0x6b03;
pub struct Endpoint { channel: Option<fl::Handle>, terminal: Option<Operation> }
impl Endpoint {
    pub fn new() -> Self { Self { channel: None, terminal: None } }
    pub fn handle(&mut self, m: IpcMsg) -> bool {
        if m.label == BIND {
            let h = fl::Handle { token: m.words[0], generation: m.words[1] };
            assert_eq!(fl::operate(35, h, 0x10010), Ok(1));
            assert!(self.channel.is_none());
            self.channel = Some(h);
            self.terminal = Some(Operation::committed(Transport::Ipc));
            sunlight_ipc::ipc_reply_result(IpcMsg::with_label(BIND)).unwrap();
            return true;
        }
        if m.label != 0x6b01_0010 { return false; }
        let h = self.channel.expect("bound channel");
        assert_eq!(fl::operate(35, h, 0x10010), Ok(1));
        let request = Frame::from_ipc_message(&m).expect("request");
        let response = self.terminal.as_mut().unwrap().execute(request).unwrap();
        sunlight_ipc::debug_log("[FL-M61B] case=actor-executed transport=ipc authorized=1");
        sunlight_ipc::ipc_reply_result(response.ipc_message()).unwrap();
        true
    }
}
pub fn supervisor() -> ! {
    assert_eq!(unsafe { sunlight_libc::sys::syscall1(0xf1c3, 0) }, 0);
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    let parent = alloc::format!("{}", public.0);
    let pid = sunlight_libc::spawn(b"/test/contract-system", &[b"session-racer", b"FL-actor", parent.as_bytes()], None).unwrap();
    let hello = sunlight_ipc::ipc_recv_timeout(ep, 5000).unwrap();
    assert_eq!(hello.label, HELLO); assert_eq!(hello.badge, pid);
    let actor = Actor { pid, send: hello.caps[0] };
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(HELLO)).unwrap();
    let h = offer();
    let consent = expect(actor, AUTHORIZE, h, 1, 0);
    assert_eq!(fl::operate(31, h, 0x101), Ok(1));
    assert_eq!(fl::operate(32, h, consent.words[1]), Ok(1));
    let bound = sunlight_ipc::ipc_call_timeout(actor.send, IpcMsg::with_label(BIND).word(0,h.token).word(1,h.generation),5000).unwrap();
    assert_eq!(bound.label,BIND);
    let request = Frame::request(1,b"0123456789abcdef").unwrap();
    let response = sunlight_ipc::ipc_call_timeout(actor.send,request.ipc_message(),5000).unwrap();
    assert_eq!(Frame::from_ipc_message(&response),Ok(request.transform().unwrap()));
    assert_eq!(fl::operate(4,h,0),Ok(0));
    stop(actor); zero();
    sunlight_ipc::debug_log(&alloc::format!("[FL-M61B] case=complete pid={} transport=ipc request=0123456789abcdef response=fedcba9876543210",sunlight_libc::getpid()));
    sunlight_libc::exit(0)
}
