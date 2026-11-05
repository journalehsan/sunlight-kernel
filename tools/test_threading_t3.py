"""Negative controls for actual hardware topology witnesses."""
import unittest
from threading_t3_validate import assess, hardware_profile


def witness(ids):
    rows = [f'[CPU-ID] logical={i} apic={apic} bsp={str(i == 0).lower()}' for i, apic in enumerate(ids)]
    for i, apic in enumerate(ids):
        tag = 'bsp' if i == 0 else 'online'
        rows.append(f'[CPU-ID] {tag} logical={i} apic={apic} tss={hex(100+i)} ist_df={hex(200+i)} ist_nmi={hex(300+i)}')
    return '\n'.join(rows) + '\nPASS\n'


class TopologyParser(unittest.TestCase):
    def test_valid(self):
        for ids in ([0], [0, 1], [42, 2, 128, 80]):
            self.assertTrue(assess(witness(ids), ['PASS'], len(ids)))

    def test_negatives(self):
        valid = witness([42, 2])
        for text in (valid.replace('logical=1 apic=2 bsp=false', 'logical=0 apic=2 bsp=false'),
                     valid.replace('apic=2', 'apic=42'),
                     valid.replace('apic=2', 'apic=255'),
                     valid.replace('bsp=false', 'bsp=true'),
                     valid.replace('ist_nmi=0x12d', 'ist_nmi=0x12c'),
                     valid.replace('ist_df=0xc9', 'ist_df=0x12c'),
                     valid.replace('tss=0x65', 'tss=0x64'),
                     valid.replace('online logical=1', 'online logical=0'),
                     valid.replace('PASS\n', 'PASS'), valid + 'KERNEL PANIC\n'):
            self.assertFalse(assess(text, ['PASS'], 2), text)

    def test_hardware_profiles(self):
        self.assertTrue(hardware_profile('sparse', [0,1,2,4], 4))
        self.assertTrue(hardware_profile('high', [0,64], 2))
        self.assertTrue(hardware_profile('nonzero', [42], 1))
        self.assertFalse(hardware_profile('nonzero', [0], 1))
        self.assertFalse(hardware_profile('sparse', [0,1,2,3], 4))
        self.assertFalse(hardware_profile('high', [0,1], 2))
        self.assertFalse(hardware_profile('high', [64,64], 2))
        self.assertFalse(hardware_profile('sparse', [], 4))
