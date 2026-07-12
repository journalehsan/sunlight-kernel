/// 16-bit real-mode CPU register state used by the initial interpreter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuState {
    pub ax: u16,
    pub bx: u16,
    pub cx: u16,
    pub dx: u16,
    pub si: u16,
    pub di: u16,
    pub bp: u16,
    pub sp: u16,
    pub cs: u16,
    pub ds: u16,
    pub es: u16,
    pub ss: u16,
    pub ip: u16,
    pub flags: u16,
}

impl CpuState {
    pub const fn ah(self) -> u8 {
        (self.ax >> 8) as u8
    }

    pub const fn al(self) -> u8 {
        self.ax as u8
    }

    pub const fn bh(self) -> u8 {
        (self.bx >> 8) as u8
    }

    pub const fn bl(self) -> u8 {
        self.bx as u8
    }

    pub const fn ch(self) -> u8 {
        (self.cx >> 8) as u8
    }

    pub const fn cl(self) -> u8 {
        self.cx as u8
    }

    pub const fn dh(self) -> u8 {
        (self.dx >> 8) as u8
    }

    pub const fn dl(self) -> u8 {
        self.dx as u8
    }

    pub fn set_ah(&mut self, value: u8) {
        self.ax = (self.ax & 0x00ff) | ((value as u16) << 8);
    }

    pub fn set_al(&mut self, value: u8) {
        self.ax = (self.ax & 0xff00) | value as u16;
    }

    pub fn set_bh(&mut self, value: u8) {
        self.bx = (self.bx & 0x00ff) | ((value as u16) << 8);
    }

    pub fn set_bl(&mut self, value: u8) {
        self.bx = (self.bx & 0xff00) | value as u16;
    }

    pub fn set_ch(&mut self, value: u8) {
        self.cx = (self.cx & 0x00ff) | ((value as u16) << 8);
    }

    pub fn set_cl(&mut self, value: u8) {
        self.cx = (self.cx & 0xff00) | value as u16;
    }

    pub fn set_dh(&mut self, value: u8) {
        self.dx = (self.dx & 0x00ff) | ((value as u16) << 8);
    }

    pub fn set_dl(&mut self, value: u8) {
        self.dx = (self.dx & 0xff00) | value as u16;
    }

    pub fn set_reg8(&mut self, index: u8, value: u8) {
        match index {
            0 => self.set_al(value),
            1 => self.set_cl(value),
            2 => self.set_dl(value),
            3 => self.set_bl(value),
            4 => self.set_ah(value),
            5 => self.set_ch(value),
            6 => self.set_dh(value),
            7 => self.set_bh(value),
            _ => unreachable!("three-bit register encoding"),
        }
    }

    pub fn set_reg16(&mut self, index: u8, value: u16) {
        match index {
            0 => self.ax = value,
            1 => self.cx = value,
            2 => self.dx = value,
            3 => self.bx = value,
            4 => self.sp = value,
            5 => self.bp = value,
            6 => self.si = value,
            7 => self.di = value,
            _ => unreachable!("three-bit register encoding"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CpuState;

    #[test]
    fn eight_bit_register_halves_preserve_the_other_half() {
        let mut cpu = CpuState {
            ax: 0x1234,
            bx: 0x5678,
            cx: 0x9abc,
            dx: 0xdef0,
            ..CpuState::default()
        };
        cpu.set_ah(0xfe);
        cpu.set_bl(0xed);
        cpu.set_ch(0xbe);
        cpu.set_dl(0xef);

        assert_eq!(cpu.ax, 0xfe34);
        assert_eq!(cpu.bx, 0x56ed);
        assert_eq!(cpu.cx, 0xbebc);
        assert_eq!(cpu.dx, 0xdeef);
        assert_eq!((cpu.ah(), cpu.al()), (0xfe, 0x34));
    }
}
