set verbose on
set osabi none

define target hookpost-remote

python
import gdb
rip = int(gdb.parse_and_eval("(unsigned long long)$rip"))
base_address = rip & 0xfffffffffffff000
while int(gdb.parse_and_eval(f"*(unsigned short *){base_address}")) != 0x5a4d:
    base_address -= 0x1000
print(f"Found PE Base Address: {hex(base_address)}")

image_base = 0x140000000
offset = base_address - image_base
filepath = gdb.current_progspace().filename
print(f"Relocating IDE-loaded file: {filepath}")
gdb.execute("set confirm off")
gdb.execute(f"symbol-file {filepath} -o {offset}")
gdb.execute("set confirm on")
end

    set language c
    set *(unsigned long long*)&GDB_ATTACHED = 1
    set language rust
end