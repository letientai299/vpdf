import ctypes
import os
import sys

RENAME_SWAP = 2


def replace(source, destination):
    if not os.path.lexists(destination):
        os.replace(source, destination)
        return
    library = ctypes.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True)
    rename = library.renamex_np
    rename.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
    rename.restype = ctypes.c_int
    # Swap preserves the destination on failure.
    if rename(os.fsencode(source), os.fsencode(destination), RENAME_SWAP) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), destination)


if __name__ == "__main__":
    replace(*sys.argv[1:])
