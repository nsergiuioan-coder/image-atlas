import ctypes
from ctypes import wintypes
from pathlib import Path
import struct
import sys


def verify(executable, icon):
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.LoadLibraryExW.argtypes = [wintypes.LPCWSTR, wintypes.HANDLE, wintypes.DWORD]
    kernel.LoadLibraryExW.restype = wintypes.HMODULE
    kernel.FindResourceW.argtypes = [wintypes.HMODULE, ctypes.c_void_p, ctypes.c_void_p]
    kernel.FindResourceW.restype = wintypes.HRSRC
    kernel.SizeofResource.argtypes = [wintypes.HMODULE, wintypes.HRSRC]
    kernel.SizeofResource.restype = wintypes.DWORD
    kernel.LoadResource.argtypes = [wintypes.HMODULE, wintypes.HRSRC]
    kernel.LoadResource.restype = wintypes.HGLOBAL
    kernel.LockResource.argtypes = [wintypes.HGLOBAL]
    kernel.LockResource.restype = ctypes.c_void_p
    kernel.FreeLibrary.argtypes = [wintypes.HMODULE]
    kernel.FreeLibrary.restype = wintypes.BOOL
    enum_callback = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HMODULE, ctypes.c_void_p, ctypes.c_void_p, wintypes.LPARAM)
    kernel.EnumResourceNamesW.argtypes = [wintypes.HMODULE, ctypes.c_void_p, enum_callback, wintypes.LPARAM]
    kernel.EnumResourceNamesW.restype = wintypes.BOOL
    module = kernel.LoadLibraryExW(str(executable.resolve()), None, 0x22)
    if not module:
        raise ctypes.WinError(ctypes.get_last_error())

    def resource(kind, identifier):
        entry = kernel.FindResourceW(module, identifier, kind)
        if not entry:
            raise RuntimeError(f'{executable.name}: missing resource {kind}/{identifier}')
        length = kernel.SizeofResource(module, entry)
        data = kernel.LockResource(kernel.LoadResource(module, entry))
        if not data or not length:
            raise RuntimeError(f'{executable.name}: empty resource {kind}/{identifier}')
        return ctypes.string_at(data, length)

    try:
        source = icon.read_bytes()
        groups = []

        @enum_callback
        def collect_group(_module, _kind, name, _parameter):
            groups.append(name)
            return True

        if not kernel.EnumResourceNamesW(module, 14, collect_group, 0) or not groups:
            raise RuntimeError(f'{executable.name}: no embedded icon')
        group = resource(14, groups[0])
        assert group[:6] == source[:6], 'Icon count or type differs'
        count = struct.unpack_from('<H', source, 4)[0]
        for index in range(count):
            expected = struct.unpack_from('<BBBBHHII', source, 6 + index * 16)
            embedded = struct.unpack_from('<BBBBHHIH', group, 6 + index * 14)
            assert expected[:2] == embedded[:2], 'Icon dimensions differ'
            assert expected[6] == embedded[6], 'Icon byte length differs'
            start = expected[7]
            assert resource(3, embedded[7]) == source[start:start + expected[6]], 'Icon pixels differ'
        print(f'{executable.name}: all {count} Atlas icon sizes verified')
    finally:
        kernel.FreeLibrary(module)


if __name__ == '__main__':
    verify(Path(sys.argv[1]), Path(sys.argv[2]))
