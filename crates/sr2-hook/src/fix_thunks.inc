// Thunks that keep XMM0-7 and (by default) ECX/EDX, and hand the Rust body an out pointer (last
// argument) to a [ecx, edx] pair it may overwrite to leave the original's exit values.
#[allow(unused_macros)]
macro_rules! kx_thunk0 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "sub esp, 8",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "lea eax, [esp + 128]",
                "push eax",
                "call {f}",
                "add esp, 4",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 8",
                "ret",
                f = sym $f,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kx_thunk1 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "sub esp, 8",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + 140]",
                "lea eax, [esp + 132]",
                "push eax",
                "call {f}",
                "add esp, 8",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 8",
                "ret",
                f = sym $f,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kx_thunk2 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "sub esp, 8",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + 144]",
                "push dword ptr [esp + 144]",
                "lea eax, [esp + 136]",
                "push eax",
                "call {f}",
                "add esp, 12",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 8",
                "ret",
                f = sym $f,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kx_thunk3 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "sub esp, 8",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + 148]",
                "push dword ptr [esp + 148]",
                "push dword ptr [esp + 148]",
                "lea eax, [esp + 140]",
                "push eax",
                "call {f}",
                "add esp, 16",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 8",
                "ret",
                f = sym $f,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kx_thunk4 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "sub esp, 8",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + 152]",
                "push dword ptr [esp + 152]",
                "push dword ptr [esp + 152]",
                "push dword ptr [esp + 152]",
                "lea eax, [esp + 144]",
                "push eax",
                "call {f}",
                "add esp, 20",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 8",
                "ret",
                f = sym $f,
            )
        }
    };
}

// kxs<N>!: like keep_ecx_edx (ECX/EDX kept) for a Rust fn with N stack args and NO out pointer, but also keeps XMM0-7.
#[allow(unused_macros)]
macro_rules! kxs0 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "push ecx",
                "push edx",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "call {f}",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "pop edx",
                "pop ecx",
                "ret",
                f = sym $f,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kxs1 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "push ecx",
                "push edx",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + 140]",
                "call {f}",
                "add esp, 4",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "pop edx",
                "pop ecx",
                "ret",
                f = sym $f,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kxs2 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "push ecx",
                "push edx",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + 144]",
                "push dword ptr [esp + 144]",
                "call {f}",
                "add esp, 8",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "pop edx",
                "pop ecx",
                "ret",
                f = sym $f,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kxs3 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "push ecx",
                "push edx",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + 148]",
                "push dword ptr [esp + 148]",
                "push dword ptr [esp + 148]",
                "call {f}",
                "add esp, 12",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "pop edx",
                "pop ecx",
                "ret",
                f = sym $f,
            )
        }
    };
}
