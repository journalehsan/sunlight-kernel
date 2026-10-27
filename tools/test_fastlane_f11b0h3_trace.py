import unittest
from fastlane_f11b0h3_trace import assess

class TraceTests(unittest.TestCase):
    def test_missing(self):
        self.assertEqual(assess('')['classification'], 'NOT_RUN')
    def test_bounded_observation_is_not_security_proof(self):
        s = '[H3-TRACE] schema=1 count=1 capacity=256 overflow=0\n[H3-TRACE] schema=1 seq=1 event=1 cpu=0 pid=9 inc=3 a=4 b=1 c=1\n'
        result = assess(s)
        self.assertEqual(result['classification'], 'OBSERVATIONS_ONLY')
        self.assertEqual(result['security_acceptance'], 'NOT_FORCED')
        for broken in (s.replace('overflow=0', 'overflow=1'), s.replace('seq=1', 'seq=2'), s.replace('event=1', 'event=0'), s.replace('inc=3', 'inc=0'), s.replace('count=1', 'count=2'), s.replace('schema=1', 'schema=2', 1)):
            self.assertEqual(assess(broken)['classification'], 'FAIL_CURRENT')

if __name__ == '__main__':
    unittest.main()
