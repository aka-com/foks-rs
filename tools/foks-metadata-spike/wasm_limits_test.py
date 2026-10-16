import unittest
from wasm_limits import memory_limits

class Limits(unittest.TestCase):
    def test_declared_maximum(self):
        self.assertEqual(memory_limits(b'\0asm\1\0\0\0\5\5\1\1\1\x80\x08'),(1,1024))
    def test_missing_or_unbounded(self):
        for data in [b'\0asm\1\0\0\0',b'\0asm\1\0\0\0\5\3\1\0\1',b'bad']:
            with self.assertRaises(ValueError):memory_limits(data)
    def test_excessive_or_truncated(self):
        for data in [b'\0asm\1\0\0\0\5\5\1\1\1\x81\x08',b'\0asm\1\0\0\0\5\5\1']:
            with self.assertRaises(ValueError):memory_limits(data)

if __name__=='__main__':unittest.main()
