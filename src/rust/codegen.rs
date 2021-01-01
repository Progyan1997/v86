use cpu::BitSize;
use cpu2::cpu::{
    FLAG_CARRY, FLAG_ZERO, TLB_GLOBAL, TLB_HAS_CODE, TLB_NO_USER, TLB_READONLY, TLB_VALID,
};
use cpu2::imports::mem8;
use global_pointers;
use jit::JitContext;
use jit_instructions::LocalOrImmedate;
use modrm;
use profiler;
use regs;
use wasmgen::wasm_builder;
use wasmgen::wasm_builder::{WasmBuilder, WasmLocal, WasmLocalI64};

const CONDITION_FUNCTIONS: [&str; 16] = [
    "test_o", "test_no", "test_b", "test_nb", "test_z", "test_nz", "test_be", "test_nbe", "test_s",
    "test_ns", "test_p", "test_np", "test_l", "test_nl", "test_le", "test_nle",
];

pub fn gen_add_cs_offset(ctx: &mut JitContext) {
    ctx.builder
        .load_aligned_i32(global_pointers::get_seg_offset(regs::CS));
    ctx.builder.add_i32();
}

pub fn gen_set_previous_eip_offset_from_eip(builder: &mut WasmBuilder, n: u32) {
    // previous_ip = instruction_pointer + n
    builder.const_i32(global_pointers::PREVIOUS_IP as i32);
    builder.load_aligned_i32(global_pointers::INSTRUCTION_POINTER);
    if n != 0 {
        builder.const_i32(n as i32);
        builder.add_i32();
    }
    builder.store_aligned_i32(0);
}

pub fn gen_set_previous_eip_offset_from_eip_with_low_bits(
    builder: &mut WasmBuilder,
    low_bits: i32,
) {
    // previous_ip = instruction_pointer & ~0xFFF | low_bits;
    builder.const_i32(global_pointers::PREVIOUS_IP as i32);
    builder.load_aligned_i32(global_pointers::INSTRUCTION_POINTER);
    builder.const_i32(!0xFFF);
    builder.and_i32();
    builder.const_i32(low_bits);
    builder.or_i32();
    builder.store_aligned_i32(0);
}

pub fn gen_increment_instruction_pointer(builder: &mut WasmBuilder, n: u32) {
    builder.const_i32(global_pointers::INSTRUCTION_POINTER as i32);
    builder.load_aligned_i32(global_pointers::INSTRUCTION_POINTER);
    builder.const_i32(n as i32);
    builder.add_i32();
    builder.store_aligned_i32(0);
}

pub fn gen_relative_jump(builder: &mut WasmBuilder, n: i32) {
    // add n to instruction_pointer (without setting the offset as above)
    builder.const_i32(global_pointers::INSTRUCTION_POINTER as i32);
    builder.load_aligned_i32(global_pointers::INSTRUCTION_POINTER);
    builder.const_i32(n);
    builder.add_i32();
    builder.store_aligned_i32(0);
}

pub fn gen_set_eip(ctx: &mut JitContext, from: &WasmLocal) {
    ctx.builder
        .const_i32(global_pointers::INSTRUCTION_POINTER as i32);
    ctx.builder.get_local(&from);
    ctx.builder.store_aligned_i32(0);
}

pub fn gen_increment_variable(builder: &mut WasmBuilder, variable_address: u32, n: i32) {
    builder.increment_variable(variable_address, n);
}

pub fn gen_increment_timestamp_counter(builder: &mut WasmBuilder, n: i32) {
    gen_increment_variable(builder, global_pointers::TIMESTAMP_COUNTER, n);
}

pub fn gen_increment_mem32(builder: &mut WasmBuilder, addr: u32) { builder.increment_mem32(addr) }

pub fn gen_get_reg8(ctx: &mut JitContext, r: u32) {
    match r {
        regs::AL | regs::CL | regs::DL | regs::BL => {
            ctx.builder.get_local(&ctx.register_locals[r as usize]);
            ctx.builder.const_i32(0xFF);
            ctx.builder.and_i32();
        },
        regs::AH | regs::CH | regs::DH | regs::BH => {
            ctx.builder
                .get_local(&ctx.register_locals[(r - 4) as usize]);
            ctx.builder.const_i32(8);
            ctx.builder.shr_u_i32();
            ctx.builder.const_i32(0xFF);
            ctx.builder.and_i32();
        },
        _ => assert!(false),
    }
}

pub fn gen_get_reg16(ctx: &mut JitContext, r: u32) {
    ctx.builder.get_local(&ctx.register_locals[r as usize]);
    ctx.builder.const_i32(0xFFFF);
    ctx.builder.and_i32();
}

pub fn gen_get_reg32(ctx: &mut JitContext, r: u32) {
    ctx.builder.get_local(&ctx.register_locals[r as usize]);
}

pub fn gen_set_reg8(ctx: &mut JitContext, r: u32) {
    match r {
        regs::AL | regs::CL | regs::DL | regs::BL => {
            // reg32[r] = stack_value & 0xFF | reg32[r] & ~0xFF
            ctx.builder.const_i32(0xFF);
            ctx.builder.and_i32();

            ctx.builder.get_local(&ctx.register_locals[r as usize]);
            ctx.builder.const_i32(!0xFF);
            ctx.builder.and_i32();

            ctx.builder.or_i32();
            ctx.builder.set_local(&ctx.register_locals[r as usize]);
        },
        regs::AH | regs::CH | regs::DH | regs::BH => {
            // reg32[r] = stack_value << 8 & 0xFF00 | reg32[r] & ~0xFF00
            ctx.builder.const_i32(8);
            ctx.builder.shl_i32();
            ctx.builder.const_i32(0xFF00);
            ctx.builder.and_i32();

            ctx.builder
                .get_local(&ctx.register_locals[(r - 4) as usize]);
            ctx.builder.const_i32(!0xFF00);
            ctx.builder.and_i32();

            ctx.builder.or_i32();
            ctx.builder
                .set_local(&ctx.register_locals[(r - 4) as usize]);
        },
        _ => assert!(false),
    }
}

pub fn gen_set_reg16(ctx: &mut JitContext, r: u32) {
    // reg32[r] = v & 0xFFFF | reg32[r] & ~0xFFFF

    ctx.builder.const_i32(0xFFFF);
    ctx.builder.and_i32();

    ctx.builder.get_local(&ctx.register_locals[r as usize]);
    ctx.builder.const_i32(!0xFFFF);
    ctx.builder.and_i32();

    ctx.builder.or_i32();
    ctx.builder.set_local(&ctx.register_locals[r as usize]);
}

pub fn gen_set_reg32(ctx: &mut JitContext, r: u32) {
    ctx.builder.set_local(&ctx.register_locals[r as usize]);
}

pub fn gen_get_sreg(ctx: &mut JitContext, r: u32) {
    ctx.builder
        .load_aligned_u16(global_pointers::get_sreg_offset(r));
}

/// sign-extend a byte value on the stack and leave it on the stack
pub fn sign_extend_i8(builder: &mut WasmBuilder) {
    builder.const_i32(24);
    builder.shl_i32();
    builder.const_i32(24);
    builder.shr_s_i32();
}

/// sign-extend a two byte value on the stack and leave it on the stack
pub fn sign_extend_i16(builder: &mut WasmBuilder) {
    builder.const_i32(16);
    builder.shl_i32();
    builder.const_i32(16);
    builder.shr_s_i32();
}

pub fn gen_fn0_const(builder: &mut WasmBuilder, name: &str) {
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN0_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_fn0_const_ret(builder: &mut WasmBuilder, name: &str) {
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN0_RET_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_fn1_const(builder: &mut WasmBuilder, name: &str, arg0: u32) {
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN1_TYPE_INDEX);
    builder.const_i32(arg0 as i32);
    builder.call_fn(fn_idx);
}

pub fn gen_call_fn1_ret(builder: &mut WasmBuilder, name: &str) {
    // generates: fn( _ ) where _ must be left on the stack before calling this, and fn returns a value
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN1_RET_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_call_fn1_ret_f64(builder: &mut WasmBuilder, name: &str) {
    // generates: fn( _ ) where _ must be left on the stack before calling this, and fn returns a value
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN1_RET_F64_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_call_fn1_f64_ret_i32(builder: &mut WasmBuilder, name: &str) {
    // generates: fn( _ ) where _ must be left on the stack before calling this, and fn returns a value
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN1_F64_RET_I32_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_call_fn1_f64_ret_i64(builder: &mut WasmBuilder, name: &str) {
    // generates: fn( _ ) where _ must be left on the stack before calling this, and fn returns a value
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN1_F64_RET_I64_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_call_fn1_ret_i64(builder: &mut WasmBuilder, name: &str) {
    // generates: fn( _ ) where _ must be left on the stack before calling this, and fn returns a value
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN1_RET_I64_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_fn2_const(builder: &mut WasmBuilder, name: &str, arg0: u32, arg1: u32) {
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN2_TYPE_INDEX);
    builder.const_i32(arg0 as i32);
    builder.const_i32(arg1 as i32);
    builder.call_fn(fn_idx);
}

pub fn gen_call_fn1(builder: &mut WasmBuilder, name: &str) {
    // generates: fn( _ ) where _ must be left on the stack before calling this
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN1_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_call_fn2(builder: &mut WasmBuilder, name: &str) {
    // generates: fn( _, _ ) where _ must be left on the stack before calling this
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN2_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_call_fn2_i32_f64(builder: &mut WasmBuilder, name: &str) {
    // generates: fn( _, _ ) where _ must be left on the stack before calling this
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN2_I32_F64_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_call_fn2_i32_i64(builder: &mut WasmBuilder, name: &str) {
    // generates: fn( _, _ ) where _ must be left on the stack before calling this
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN2_I32_I64_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_call_fn1_f64(builder: &mut WasmBuilder, name: &str) {
    // generates: fn( _, _ ) where _ must be left on the stack before calling this
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN1_F64_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_call_fn2_ret(builder: &mut WasmBuilder, name: &str) {
    // generates: fn( _, _ ) where _ must be left on the stack before calling this, and fn returns a value
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN2_RET_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_call_fn3(builder: &mut WasmBuilder, name: &str) {
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN3_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_call_fn3_i32_i64_i64(builder: &mut WasmBuilder, name: &str) {
    // generates: fn( _, _ ) where _ must be left on the stack before calling this
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN3_I32_I64_I64_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_call_fn3_ret(builder: &mut WasmBuilder, name: &str) {
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN3_RET_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_fn3_const(builder: &mut WasmBuilder, name: &str, arg0: u32, arg1: u32, arg2: u32) {
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN3_TYPE_INDEX);
    builder.const_i32(arg0 as i32);
    builder.const_i32(arg1 as i32);
    builder.const_i32(arg2 as i32);
    builder.call_fn(fn_idx);
}

pub fn gen_modrm_fn0(builder: &mut WasmBuilder, name: &str) {
    // generates: fn( _ )
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN1_TYPE_INDEX);
    builder.call_fn(fn_idx);
}

pub fn gen_modrm_fn1(builder: &mut WasmBuilder, name: &str, arg0: u32) {
    // generates: fn( _, arg0 )
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN2_TYPE_INDEX);
    builder.const_i32(arg0 as i32);
    builder.call_fn(fn_idx);
}

pub fn gen_modrm_fn2(builder: &mut WasmBuilder, name: &str, arg0: u32, arg1: u32) {
    // generates: fn( _, arg0, arg1 )
    let fn_idx = builder.get_fn_idx(name, wasm_builder::FN3_TYPE_INDEX);
    builder.const_i32(arg0 as i32);
    builder.const_i32(arg1 as i32);
    builder.call_fn(fn_idx);
}

pub fn gen_modrm_resolve(ctx: &mut JitContext, modrm_byte: u8) { modrm::gen(ctx, modrm_byte) }

pub fn gen_set_reg8_r(ctx: &mut JitContext, dest: u32, src: u32) {
    // generates: reg8[r_dest] = reg8[r_src]
    gen_get_reg8(ctx, src);
    gen_set_reg8(ctx, dest);
}
pub fn gen_set_reg16_r(ctx: &mut JitContext, dest: u32, src: u32) {
    // generates: reg16[r_dest] = reg16[r_src]
    gen_get_reg16(ctx, src);
    gen_set_reg16(ctx, dest);
}
pub fn gen_set_reg32_r(ctx: &mut JitContext, dest: u32, src: u32) {
    // generates: reg32[r_dest] = reg32[r_src]
    gen_get_reg32(ctx, src);
    gen_set_reg32(ctx, dest);
}

pub fn gen_modrm_resolve_safe_read8(ctx: &mut JitContext, modrm_byte: u8) {
    gen_modrm_resolve(ctx, modrm_byte);
    let address_local = ctx.builder.set_new_local();
    gen_safe_read8(ctx, &address_local);
    ctx.builder.free_local(address_local);
}
pub fn gen_modrm_resolve_safe_read16(ctx: &mut JitContext, modrm_byte: u8) {
    gen_modrm_resolve(ctx, modrm_byte);
    let address_local = ctx.builder.set_new_local();
    gen_safe_read16(ctx, &address_local);
    ctx.builder.free_local(address_local);
}
pub fn gen_modrm_resolve_safe_read32(ctx: &mut JitContext, modrm_byte: u8) {
    gen_modrm_resolve(ctx, modrm_byte);
    let address_local = ctx.builder.set_new_local();
    gen_safe_read32(ctx, &address_local);
    ctx.builder.free_local(address_local);
}
pub fn gen_modrm_resolve_safe_read64(ctx: &mut JitContext, modrm_byte: u8) {
    gen_modrm_resolve(ctx, modrm_byte);
    let address_local = ctx.builder.set_new_local();
    gen_safe_read64(ctx, &address_local);
    ctx.builder.free_local(address_local);
}
pub fn gen_modrm_resolve_safe_read128(ctx: &mut JitContext, modrm_byte: u8, where_to_write: u32) {
    gen_modrm_resolve(ctx, modrm_byte);
    let address_local = ctx.builder.set_new_local();
    gen_safe_read128(ctx, &address_local, where_to_write);
    ctx.builder.free_local(address_local);
}

pub fn gen_safe_read8(ctx: &mut JitContext, address_local: &WasmLocal) {
    gen_safe_read(ctx, BitSize::BYTE, address_local, None);
}
pub fn gen_safe_read16(ctx: &mut JitContext, address_local: &WasmLocal) {
    gen_safe_read(ctx, BitSize::WORD, address_local, None);
}
pub fn gen_safe_read32(ctx: &mut JitContext, address_local: &WasmLocal) {
    gen_safe_read(ctx, BitSize::DWORD, address_local, None);
}
pub fn gen_safe_read64(ctx: &mut JitContext, address_local: &WasmLocal) {
    gen_safe_read(ctx, BitSize::QWORD, &address_local, None);
}
pub fn gen_safe_read128(ctx: &mut JitContext, address_local: &WasmLocal, where_to_write: u32) {
    gen_safe_read(ctx, BitSize::DQWORD, &address_local, Some(where_to_write));
}

// only used internally for gen_safe_write
enum GenSafeWriteValue<'a> {
    I32(&'a WasmLocal),
    I64(&'a WasmLocalI64),
    TwoI64s(&'a WasmLocalI64, &'a WasmLocalI64),
}

pub fn gen_safe_write8(ctx: &mut JitContext, address_local: &WasmLocal, value_local: &WasmLocal) {
    gen_safe_write(
        ctx,
        BitSize::BYTE,
        address_local,
        GenSafeWriteValue::I32(value_local),
    )
}
pub fn gen_safe_write16(ctx: &mut JitContext, address_local: &WasmLocal, value_local: &WasmLocal) {
    gen_safe_write(
        ctx,
        BitSize::WORD,
        address_local,
        GenSafeWriteValue::I32(value_local),
    )
}
pub fn gen_safe_write32(ctx: &mut JitContext, address_local: &WasmLocal, value_local: &WasmLocal) {
    gen_safe_write(
        ctx,
        BitSize::DWORD,
        address_local,
        GenSafeWriteValue::I32(value_local),
    )
}
pub fn gen_safe_write64(
    ctx: &mut JitContext,
    address_local: &WasmLocal,
    value_local: &WasmLocalI64,
) {
    gen_safe_write(
        ctx,
        BitSize::QWORD,
        address_local,
        GenSafeWriteValue::I64(value_local),
    )
}

pub fn gen_safe_write128(
    ctx: &mut JitContext,
    address_local: &WasmLocal,
    value_local_low: &WasmLocalI64,
    value_local_high: &WasmLocalI64,
) {
    gen_safe_write(
        ctx,
        BitSize::DQWORD,
        address_local,
        GenSafeWriteValue::TwoI64s(value_local_low, value_local_high),
    )
}

fn gen_safe_read(
    ctx: &mut JitContext,
    bits: BitSize,
    address_local: &WasmLocal,
    where_to_write: Option<u32>,
) {
    // Assumes virtual address has been pushed to the stack, and generates safe_readXX's fast-path
    // inline, bailing to safe_readXX_slow if necessary

    ctx.builder.get_local(&address_local);

    // Pseudo: base_on_stack = (uint32_t)address >> 12;
    ctx.builder.const_i32(12);
    ctx.builder.shr_u_i32();

    // scale index
    ctx.builder.const_i32(2);
    ctx.builder.shl_i32();

    // Pseudo: entry = tlb_data[base_on_stack];
    ctx.builder
        .load_aligned_i32_from_stack(global_pointers::TLB_DATA);
    let entry_local = ctx.builder.tee_new_local();

    // Pseudo: bool can_use_fast_path =
    //    (entry & 0xFFF & ~TLB_READONLY & ~TLB_GLOBAL & ~TLB_HAS_CODE & ~(cpl == 3 ? 0 : TLB_NO_USER) == TLB_VALID &&
    //    (bitsize == 8 ? true : (address & 0xFFF) <= (0x1000 - (bitsize / 8)));
    ctx.builder.const_i32(
        (0xFFF
            & !TLB_READONLY
            & !TLB_GLOBAL
            & !TLB_HAS_CODE
            & !(if ctx.cpu.cpl3() { 0 } else { TLB_NO_USER })) as i32,
    );
    ctx.builder.and_i32();

    ctx.builder.const_i32(TLB_VALID as i32);
    ctx.builder.eq_i32();

    if bits != BitSize::BYTE {
        ctx.builder.get_local(&address_local);
        ctx.builder.const_i32(0xFFF);
        ctx.builder.and_i32();
        ctx.builder.const_i32(0x1000 - bits.bytes() as i32);
        ctx.builder.le_i32();

        ctx.builder.and_i32();
    }

    // Pseudo:
    // if(can_use_fast_path) leave_on_stack(mem8[entry & ~0xFFF ^ address]);
    if bits == BitSize::DQWORD {
        ctx.builder.if_void();
    }
    else if bits == BitSize::QWORD {
        ctx.builder.if_i64();
    }
    else {
        ctx.builder.if_i32();
    }

    gen_profiler_stat_increment(ctx.builder, profiler::stat::SAFE_READ_FAST);

    ctx.builder.get_local(&entry_local);
    ctx.builder.const_i32(!0xFFF);
    ctx.builder.and_i32();
    ctx.builder.get_local(&address_local);
    ctx.builder.xor_i32();

    // where_to_write is only used by dqword
    dbg_assert!((where_to_write != None) == (bits == BitSize::DQWORD));

    match bits {
        BitSize::BYTE => {
            ctx.builder.load_u8_from_stack(unsafe { mem8 } as u32);
        },
        BitSize::WORD => {
            ctx.builder
                .load_unaligned_u16_from_stack(unsafe { mem8 } as u32);
        },
        BitSize::DWORD => {
            ctx.builder
                .load_unaligned_i32_from_stack(unsafe { mem8 } as u32);
        },
        BitSize::QWORD => {
            ctx.builder
                .load_unaligned_i64_from_stack(unsafe { mem8 } as u32);
        },
        BitSize::DQWORD => {
            let where_to_write = where_to_write.unwrap();
            let virt_address_local = ctx.builder.set_new_local();
            ctx.builder.const_i32(0);
            ctx.builder.get_local(&virt_address_local);
            ctx.builder
                .load_unaligned_i64_from_stack(unsafe { mem8 } as u32);
            ctx.builder.store_unaligned_i64(where_to_write);

            ctx.builder.const_i32(0);
            ctx.builder.get_local(&virt_address_local);
            ctx.builder
                .load_unaligned_i64_from_stack(unsafe { mem8 } as u32 + 8);
            ctx.builder.store_unaligned_i64(where_to_write + 8);

            ctx.builder.free_local(virt_address_local);
        },
    }

    // Pseudo:
    // else {
    //     *previous_ip = *instruction_pointer & ~0xFFF | start_of_instruction;
    //     leave_on_stack(safe_read*_slow_jit(address));
    //     if(page_fault) { trigger_pagefault_end_jit(); return; }
    // }
    ctx.builder.else_();

    if cfg!(feature = "profiler") && cfg!(feature = "profiler_instrument") {
        ctx.builder.get_local(&address_local);
        ctx.builder.get_local(&entry_local);
        gen_call_fn2(ctx.builder, "report_safe_read_jit_slow");
    }

    ctx.builder.get_local(&address_local);
    match bits {
        BitSize::BYTE => {
            gen_call_fn1_ret(ctx.builder, "safe_read8_slow_jit");
        },
        BitSize::WORD => {
            gen_call_fn1_ret(ctx.builder, "safe_read16_slow_jit");
        },
        BitSize::DWORD => {
            gen_call_fn1_ret(ctx.builder, "safe_read32s_slow_jit");
        },
        BitSize::QWORD => {
            gen_call_fn1_ret_i64(ctx.builder, "safe_read64s_slow_jit");
        },
        BitSize::DQWORD => {
            ctx.builder.const_i32(where_to_write.unwrap() as i32);
            gen_call_fn2(ctx.builder, "safe_read128s_slow_jit");
        },
    }

    ctx.builder.load_u8(global_pointers::PAGE_FAULT);

    ctx.builder.if_void();
    gen_debug_track_jit_exit(ctx.builder, ctx.start_of_current_instruction);

    gen_set_previous_eip_offset_from_eip_with_low_bits(
        ctx.builder,
        ctx.start_of_current_instruction as i32 & 0xFFF,
    );

    // -2 for the exit-with-pagefault block, +2 for leaving the two nested ifs from this function
    let br_offset = ctx.current_brtable_depth - 2 + 2;
    ctx.builder.br(br_offset);
    ctx.builder.block_end();

    ctx.builder.block_end();

    ctx.builder.free_local(entry_local);
}

fn gen_safe_write(
    ctx: &mut JitContext,
    bits: BitSize,
    address_local: &WasmLocal,
    value_local: GenSafeWriteValue,
) {
    // Generates safe_writeXX' fast-path inline, bailing to safe_writeXX_slow if necessary.

    ctx.builder.get_local(&address_local);

    // Pseudo: base_on_stack = (uint32_t)address >> 12;
    ctx.builder.const_i32(12);
    ctx.builder.shr_u_i32();

    // scale index
    ctx.builder.const_i32(2);
    ctx.builder.shl_i32();

    // Pseudo: entry = tlb_data[base_on_stack];
    ctx.builder
        .load_aligned_i32_from_stack(global_pointers::TLB_DATA);
    let entry_local = ctx.builder.tee_new_local();

    // Pseudo: bool can_use_fast_path = (entry & 0xFFF & ~TLB_GLOBAL & ~(cpl == 3 ? 0 : TLB_NO_USER) == TLB_VALID &&
    //                                   (address & 0xFFF) <= (0x1000 - bitsize / 8));
    ctx.builder
        .const_i32((0xFFF & !TLB_GLOBAL & !(if ctx.cpu.cpl3() { 0 } else { TLB_NO_USER })) as i32);
    ctx.builder.and_i32();

    ctx.builder.const_i32(TLB_VALID as i32);
    ctx.builder.eq_i32();

    if bits != BitSize::BYTE {
        ctx.builder.get_local(&address_local);
        ctx.builder.const_i32(0xFFF);
        ctx.builder.and_i32();
        ctx.builder.const_i32(0x1000 - bits.bytes() as i32);
        ctx.builder.le_i32();

        ctx.builder.and_i32();
    }

    // Pseudo:
    // if(can_use_fast_path)
    // {
    //     phys_addr = entry & ~0xFFF ^ address;
    ctx.builder.if_void();

    gen_profiler_stat_increment(ctx.builder, profiler::stat::SAFE_WRITE_FAST);

    ctx.builder.get_local(&entry_local);
    ctx.builder.const_i32(!0xFFF);
    ctx.builder.and_i32();
    ctx.builder.get_local(&address_local);
    ctx.builder.xor_i32();

    // Pseudo:
    //     /* continued within can_use_fast_path branch */
    //     mem8[phys_addr] = value;

    match value_local {
        GenSafeWriteValue::I32(local) => ctx.builder.get_local(local),
        GenSafeWriteValue::I64(local) => ctx.builder.get_local_i64(local),
        GenSafeWriteValue::TwoI64s(local1, local2) => {
            assert!(bits == BitSize::DQWORD);

            let virt_address_local = ctx.builder.tee_new_local();
            ctx.builder.get_local_i64(local1);
            ctx.builder.store_unaligned_i64(unsafe { mem8 } as u32);

            ctx.builder.get_local(&virt_address_local);
            ctx.builder.get_local_i64(local2);
            ctx.builder.store_unaligned_i64(unsafe { mem8 } as u32 + 8);
            ctx.builder.free_local(virt_address_local);
        },
    }
    match bits {
        BitSize::BYTE => {
            ctx.builder.store_u8(unsafe { mem8 } as u32);
        },
        BitSize::WORD => {
            ctx.builder.store_unaligned_u16(unsafe { mem8 } as u32);
        },
        BitSize::DWORD => {
            ctx.builder.store_unaligned_i32(unsafe { mem8 } as u32);
        },
        BitSize::QWORD => {
            ctx.builder.store_unaligned_i64(unsafe { mem8 } as u32);
        },
        BitSize::DQWORD => {}, // handled above
    }

    // Pseudo:
    // else {
    //     *previous_ip = *instruction_pointer & ~0xFFF | start_of_instruction;
    //     safe_write*_slow_jit(address, value);
    //     if(page_fault) { trigger_pagefault_end_jit(); return; }
    // }
    ctx.builder.else_();

    if cfg!(feature = "profiler") && cfg!(feature = "profiler_instrument") {
        ctx.builder.get_local(&address_local);
        ctx.builder.get_local(&entry_local);
        gen_call_fn2(ctx.builder, "report_safe_write_jit_slow");
    }

    ctx.builder.get_local(&address_local);
    match value_local {
        GenSafeWriteValue::I32(local) => ctx.builder.get_local(local),
        GenSafeWriteValue::I64(local) => ctx.builder.get_local_i64(local),
        GenSafeWriteValue::TwoI64s(local1, local2) => {
            ctx.builder.get_local_i64(local1);
            ctx.builder.get_local_i64(local2)
        },
    }
    match bits {
        BitSize::BYTE => {
            gen_call_fn2(ctx.builder, "safe_write8_slow_jit");
        },
        BitSize::WORD => {
            gen_call_fn2(ctx.builder, "safe_write16_slow_jit");
        },
        BitSize::DWORD => {
            gen_call_fn2(ctx.builder, "safe_write32_slow_jit");
        },
        BitSize::QWORD => {
            gen_call_fn2_i32_i64(ctx.builder, "safe_write64_slow_jit");
        },
        BitSize::DQWORD => {
            gen_call_fn3_i32_i64_i64(ctx.builder, "safe_write128_slow_jit");
        },
    }

    ctx.builder.load_u8(global_pointers::PAGE_FAULT);

    ctx.builder.if_void();
    gen_debug_track_jit_exit(ctx.builder, ctx.start_of_current_instruction);

    gen_set_previous_eip_offset_from_eip_with_low_bits(
        ctx.builder,
        ctx.start_of_current_instruction as i32 & 0xFFF,
    );

    // -2 for the exit-with-pagefault block, +2 for leaving the two nested ifs from this function
    let br_offset = ctx.current_brtable_depth - 2 + 2;
    ctx.builder.br(br_offset);
    ctx.builder.block_end();

    ctx.builder.block_end();

    ctx.builder.free_local(entry_local);
}

pub fn gen_jmp_rel16(builder: &mut WasmBuilder, rel16: u16) {
    let cs_offset_addr = global_pointers::get_seg_offset(regs::CS);
    builder.load_aligned_i32(cs_offset_addr);
    let local = builder.set_new_local();

    // generate:
    // *instruction_pointer = cs_offset + ((*instruction_pointer - cs_offset + rel16) & 0xFFFF);
    {
        builder.const_i32(global_pointers::INSTRUCTION_POINTER as i32);

        builder.load_aligned_i32(global_pointers::INSTRUCTION_POINTER);
        builder.get_local(&local);
        builder.sub_i32();

        builder.const_i32(rel16 as i32);
        builder.add_i32();

        builder.const_i32(0xFFFF);
        builder.and_i32();

        builder.get_local(&local);
        builder.add_i32();

        builder.store_aligned_i32(0);
    }
    builder.free_local(local);
}

pub fn gen_pop16_ss16(ctx: &mut JitContext) {
    // sp = segment_offsets[SS] + reg16[SP] (or just reg16[SP] if has_flat_segmentation)
    gen_get_reg16(ctx, regs::SP);

    if !ctx.cpu.has_flat_segmentation() {
        ctx.builder
            .load_aligned_i32(global_pointers::get_seg_offset(regs::SS));
        ctx.builder.add_i32();
    }

    // result = safe_read16(sp)
    let address_local = ctx.builder.set_new_local();
    gen_safe_read16(ctx, &address_local);
    ctx.builder.free_local(address_local);

    // reg16[SP] += 2;
    gen_get_reg16(ctx, regs::SP);
    ctx.builder.const_i32(2);
    ctx.builder.add_i32();
    gen_set_reg16(ctx, regs::SP);

    // return value is already on stack
}

pub fn gen_pop16_ss32(ctx: &mut JitContext) {
    // esp = segment_offsets[SS] + reg32[ESP] (or just reg32[ESP] if has_flat_segmentation)
    gen_get_reg32(ctx, regs::ESP);

    if !ctx.cpu.has_flat_segmentation() {
        ctx.builder
            .load_aligned_i32(global_pointers::get_seg_offset(regs::SS));
        ctx.builder.add_i32();
    }

    // result = safe_read16(esp)
    let address_local = ctx.builder.set_new_local();
    gen_safe_read16(ctx, &address_local);
    ctx.builder.free_local(address_local);

    // reg32[ESP] += 2;
    gen_get_reg32(ctx, regs::ESP);
    ctx.builder.const_i32(2);
    ctx.builder.add_i32();
    gen_set_reg32(ctx, regs::ESP);

    // return value is already on stack
}

pub fn gen_pop16(ctx: &mut JitContext) {
    if ctx.cpu.ssize_32() {
        gen_pop16_ss32(ctx);
    }
    else {
        gen_pop16_ss16(ctx);
    }
}

pub fn gen_pop32s_ss16(ctx: &mut JitContext) {
    // sp = reg16[SP]
    gen_get_reg16(ctx, regs::SP);

    // result = safe_read32s(segment_offsets[SS] + sp) (or just sp if has_flat_segmentation)
    if !ctx.cpu.has_flat_segmentation() {
        ctx.builder
            .load_aligned_i32(global_pointers::get_seg_offset(regs::SS));
        ctx.builder.add_i32();
    }

    let address_local = ctx.builder.set_new_local();
    gen_safe_read32(ctx, &address_local);
    ctx.builder.free_local(address_local);

    // reg16[SP] = sp + 4;
    gen_get_reg16(ctx, regs::SP);
    ctx.builder.const_i32(4);
    ctx.builder.add_i32();
    gen_set_reg16(ctx, regs::SP);

    // return value is already on stack
}

pub fn gen_pop32s_ss32(ctx: &mut JitContext) {
    if !ctx.cpu.has_flat_segmentation() {
        gen_get_reg32(ctx, regs::ESP);
        ctx.builder
            .load_aligned_i32(global_pointers::get_seg_offset(regs::SS));
        ctx.builder.add_i32();
        let address_local = ctx.builder.set_new_local();
        gen_safe_read32(ctx, &address_local);
        ctx.builder.free_local(address_local);
    }
    else {
        let reg = ctx.register_locals[regs::ESP as usize].unsafe_clone();
        gen_safe_read32(ctx, &reg);
    }

    gen_get_reg32(ctx, regs::ESP);
    ctx.builder.const_i32(4);
    ctx.builder.add_i32();
    gen_set_reg32(ctx, regs::ESP);

    // return value is already on stack
}

pub fn gen_pop32s(ctx: &mut JitContext) {
    if ctx.cpu.ssize_32() {
        gen_pop32s_ss32(ctx);
    }
    else {
        gen_pop32s_ss16(ctx);
    }
}

pub fn gen_adjust_stack_reg(ctx: &mut JitContext, offset: u32) {
    if ctx.cpu.ssize_32() {
        gen_get_reg32(ctx, regs::ESP);
        ctx.builder.const_i32(offset as i32);
        ctx.builder.add_i32();
        gen_set_reg32(ctx, regs::ESP);
    }
    else {
        gen_get_reg16(ctx, regs::SP);
        ctx.builder.const_i32(offset as i32);
        ctx.builder.add_i32();
        gen_set_reg16(ctx, regs::SP);
    }
}

pub fn gen_leave(ctx: &mut JitContext, os32: bool) {
    // [e]bp = safe_read{16,32}([e]bp)

    if ctx.cpu.ssize_32() {
        gen_get_reg32(ctx, regs::EBP);
    }
    else {
        gen_get_reg16(ctx, regs::BP);
    }

    let old_vbp = ctx.builder.tee_new_local();

    if !ctx.cpu.has_flat_segmentation() {
        ctx.builder
            .load_aligned_i32(global_pointers::get_seg_offset(regs::SS));
        ctx.builder.add_i32();
    }
    if os32 {
        let address_local = ctx.builder.set_new_local();
        gen_safe_read32(ctx, &address_local);
        ctx.builder.free_local(address_local);
        gen_set_reg32(ctx, regs::EBP);
    }
    else {
        let address_local = ctx.builder.set_new_local();
        gen_safe_read16(ctx, &address_local);
        ctx.builder.free_local(address_local);
        gen_set_reg16(ctx, regs::BP);
    }

    // [e]sp = [e]bp + (os32 ? 4 : 2)

    if ctx.cpu.ssize_32() {
        ctx.builder.get_local(&old_vbp);
        ctx.builder.const_i32(if os32 { 4 } else { 2 });
        ctx.builder.add_i32();
        gen_set_reg32(ctx, regs::ESP);
    }
    else {
        ctx.builder.get_local(&old_vbp);
        ctx.builder.const_i32(if os32 { 4 } else { 2 });
        ctx.builder.add_i32();
        gen_set_reg16(ctx, regs::SP);
    }

    ctx.builder.free_local(old_vbp);
}

pub fn gen_task_switch_test(ctx: &mut JitContext) {
    // generate if(cr[0] & (CR0_EM | CR0_TS)) { task_switch_test_void(); return; }
    let cr0_offset = global_pointers::get_creg_offset(0);

    dbg_assert!(regs::CR0_EM | regs::CR0_TS <= 0xFF);
    ctx.builder.load_u8(cr0_offset);
    ctx.builder.const_i32((regs::CR0_EM | regs::CR0_TS) as i32);
    ctx.builder.and_i32();

    ctx.builder.if_void();

    gen_debug_track_jit_exit(ctx.builder, ctx.start_of_current_instruction);

    gen_set_previous_eip_offset_from_eip_with_low_bits(
        ctx.builder,
        ctx.start_of_current_instruction as i32 & 0xFFF,
    );

    gen_move_registers_from_locals_to_memory(ctx);
    gen_fn0_const(ctx.builder, "task_switch_test_jit");

    ctx.builder.return_();

    ctx.builder.block_end();
}

pub fn gen_task_switch_test_mmx(ctx: &mut JitContext) {
    // generate if(cr[0] & (CR0_EM | CR0_TS)) { task_switch_test_mmx_void(); return; }
    let cr0_offset = global_pointers::get_creg_offset(0);

    dbg_assert!(regs::CR0_EM | regs::CR0_TS <= 0xFF);
    ctx.builder.load_u8(cr0_offset);
    ctx.builder.const_i32((regs::CR0_EM | regs::CR0_TS) as i32);
    ctx.builder.and_i32();

    ctx.builder.if_void();

    gen_debug_track_jit_exit(ctx.builder, ctx.start_of_current_instruction);

    gen_set_previous_eip_offset_from_eip_with_low_bits(
        ctx.builder,
        ctx.start_of_current_instruction as i32 & 0xFFF,
    );

    gen_move_registers_from_locals_to_memory(ctx);
    gen_fn0_const(ctx.builder, "task_switch_test_mmx_jit");

    ctx.builder.return_();

    ctx.builder.block_end();
}

pub fn gen_push16(ctx: &mut JitContext, value_local: &WasmLocal) {
    if ctx.cpu.ssize_32() {
        gen_get_reg32(ctx, regs::ESP);
    }
    else {
        gen_get_reg16(ctx, regs::SP);
    };

    ctx.builder.const_i32(2);
    ctx.builder.sub_i32();

    let reg_updated_local = if !ctx.cpu.ssize_32() || !ctx.cpu.has_flat_segmentation() {
        let reg_updated_local = ctx.builder.tee_new_local();
        if !ctx.cpu.ssize_32() {
            ctx.builder.const_i32(0xFFFF);
            ctx.builder.and_i32();
        }

        if !ctx.cpu.has_flat_segmentation() {
            ctx.builder
                .load_aligned_i32(global_pointers::get_seg_offset(regs::SS));
            ctx.builder.add_i32();
        }

        let sp_local = ctx.builder.set_new_local();
        gen_safe_write16(ctx, &sp_local, &value_local);
        ctx.builder.free_local(sp_local);

        ctx.builder.get_local(&reg_updated_local);
        reg_updated_local
    }
    else {
        // short path: The address written to is equal to ESP/SP minus two
        let reg_updated_local = ctx.builder.tee_new_local();
        gen_safe_write16(ctx, &reg_updated_local, &value_local);
        reg_updated_local
    };

    if ctx.cpu.ssize_32() {
        gen_set_reg32(ctx, regs::ESP);
    }
    else {
        gen_set_reg16(ctx, regs::SP);
    };
    ctx.builder.free_local(reg_updated_local);
}

pub fn gen_push32(ctx: &mut JitContext, value_local: &WasmLocal) {
    if ctx.cpu.ssize_32() {
        gen_get_reg32(ctx, regs::ESP);
    }
    else {
        gen_get_reg16(ctx, regs::SP);
    };

    ctx.builder.const_i32(4);
    ctx.builder.sub_i32();

    let new_sp_local = if !ctx.cpu.ssize_32() || !ctx.cpu.has_flat_segmentation() {
        let new_sp_local = ctx.builder.tee_new_local();
        if !ctx.cpu.ssize_32() {
            ctx.builder.const_i32(0xFFFF);
            ctx.builder.and_i32();
        }

        if !ctx.cpu.has_flat_segmentation() {
            ctx.builder
                .load_aligned_i32(global_pointers::get_seg_offset(regs::SS));
            ctx.builder.add_i32();
        }

        let sp_local = ctx.builder.set_new_local();

        gen_safe_write32(ctx, &sp_local, &value_local);
        ctx.builder.free_local(sp_local);

        ctx.builder.get_local(&new_sp_local);
        new_sp_local
    }
    else {
        // short path: The address written to is equal to ESP/SP minus four
        let new_sp_local = ctx.builder.tee_new_local();
        gen_safe_write32(ctx, &new_sp_local, &value_local);
        new_sp_local
    };

    if ctx.cpu.ssize_32() {
        gen_set_reg32(ctx, regs::ESP);
    }
    else {
        gen_set_reg16(ctx, regs::SP);
    };
    ctx.builder.free_local(new_sp_local);
}

pub fn gen_get_real_eip(ctx: &mut JitContext) {
    ctx.builder
        .load_aligned_i32(global_pointers::INSTRUCTION_POINTER);
    ctx.builder
        .load_aligned_i32(global_pointers::get_seg_offset(regs::CS));
    ctx.builder.sub_i32();
}

pub fn gen_safe_read_write(
    ctx: &mut JitContext,
    bits: BitSize,
    address_local: &WasmLocal,
    f: &dyn Fn(&mut JitContext),
) {
    ctx.builder.get_local(address_local);

    // Pseudo: base_on_stack = (uint32_t)address >> 12;
    ctx.builder.const_i32(12);
    ctx.builder.shr_u_i32();

    // scale index
    ctx.builder.const_i32(2);
    ctx.builder.shl_i32();

    // Pseudo: entry = tlb_data[base_on_stack];
    ctx.builder
        .load_aligned_i32_from_stack(global_pointers::TLB_DATA);
    let entry_local = ctx.builder.tee_new_local();

    // Pseudo: bool can_use_fast_path = (entry & 0xFFF & ~TLB_READONLY & ~TLB_GLOBAL & ~(cpl == 3 ? 0 : TLB_NO_USER) == TLB_VALID &&
    //                                   (address & 0xFFF) <= (0x1000 - (bitsize / 8));
    ctx.builder
        .const_i32((0xFFF & !TLB_GLOBAL & !(if ctx.cpu.cpl3() { 0 } else { TLB_NO_USER })) as i32);
    ctx.builder.and_i32();

    ctx.builder.const_i32(TLB_VALID as i32);
    ctx.builder.eq_i32();

    if bits != BitSize::BYTE {
        ctx.builder.get_local(&address_local);
        ctx.builder.const_i32(0xFFF);
        ctx.builder.and_i32();
        ctx.builder.const_i32(0x1000 - bits.bytes() as i32);
        ctx.builder.le_i32();
        ctx.builder.and_i32();
    }

    let can_use_fast_path_local = ctx.builder.tee_new_local();

    ctx.builder.if_i32();

    gen_profiler_stat_increment(ctx.builder, profiler::stat::SAFE_READ_WRITE_FAST);

    ctx.builder.get_local(&entry_local);
    ctx.builder.const_i32(!0xFFF);
    ctx.builder.and_i32();
    ctx.builder.get_local(&address_local);
    ctx.builder.xor_i32();

    let phys_addr_local = ctx.builder.tee_new_local();

    match bits {
        BitSize::BYTE => {
            ctx.builder.load_u8_from_stack(unsafe { mem8 } as u32);
        },
        BitSize::WORD => {
            ctx.builder
                .load_unaligned_u16_from_stack(unsafe { mem8 } as u32);
        },
        BitSize::DWORD => {
            ctx.builder
                .load_unaligned_i32_from_stack(unsafe { mem8 } as u32);
        },
        BitSize::QWORD => assert!(false),  // not used
        BitSize::DQWORD => assert!(false), // not used
    }

    ctx.builder.else_();
    {
        if cfg!(feature = "profiler") && cfg!(feature = "profiler_instrument") {
            ctx.builder.get_local(&address_local);
            ctx.builder.get_local(&entry_local);
            gen_call_fn2(ctx.builder, "report_safe_read_write_jit_slow");
        }

        ctx.builder.get_local(&address_local);

        match bits {
            BitSize::BYTE => {
                gen_call_fn1_ret(ctx.builder, "safe_read_write8_slow_jit");
            },
            BitSize::WORD => {
                gen_call_fn1_ret(ctx.builder, "safe_read_write16_slow_jit");
            },
            BitSize::DWORD => {
                gen_call_fn1_ret(ctx.builder, "safe_read_write32s_slow_jit");
            },
            BitSize::QWORD => dbg_assert!(false),
            BitSize::DQWORD => dbg_assert!(false),
        }

        ctx.builder.load_u8(global_pointers::PAGE_FAULT);

        ctx.builder.if_void();
        {
            gen_debug_track_jit_exit(ctx.builder, ctx.start_of_current_instruction);

            gen_set_previous_eip_offset_from_eip_with_low_bits(
                ctx.builder,
                ctx.start_of_current_instruction as i32 & 0xFFF,
            );

            // -2 for the exit-with-pagefault block, +2 for leaving the two nested ifs from this function
            let br_offset = ctx.current_brtable_depth - 2 + 2;
            ctx.builder.br(br_offset);
        }
        ctx.builder.block_end();
    }
    ctx.builder.block_end();

    // value is now on stack

    f(ctx);
    let value_local = ctx.builder.set_new_local();

    ctx.builder.get_local(&can_use_fast_path_local);

    ctx.builder.if_void();
    {
        ctx.builder.get_local(&phys_addr_local);
        ctx.builder.get_local(&value_local);

        match bits {
            BitSize::BYTE => {
                ctx.builder.store_u8(unsafe { mem8 } as u32);
            },
            BitSize::WORD => {
                ctx.builder.store_unaligned_u16(unsafe { mem8 } as u32);
            },
            BitSize::DWORD => {
                ctx.builder.store_unaligned_i32(unsafe { mem8 } as u32);
            },
            BitSize::QWORD => dbg_assert!(false),
            BitSize::DQWORD => dbg_assert!(false),
        }
    }
    ctx.builder.else_();
    {
        ctx.builder.get_local(&address_local);
        ctx.builder.get_local(&value_local);

        match bits {
            BitSize::BYTE => {
                gen_call_fn2(ctx.builder, "safe_write8_slow_jit");
            },
            BitSize::WORD => {
                gen_call_fn2(ctx.builder, "safe_write16_slow_jit");
            },
            BitSize::DWORD => {
                gen_call_fn2(ctx.builder, "safe_write32_slow_jit");
            },
            BitSize::QWORD => dbg_assert!(false),
            BitSize::DQWORD => dbg_assert!(false),
        }

        ctx.builder.load_u8(global_pointers::PAGE_FAULT);

        ctx.builder.if_void();
        {
            // handled above
            //ctx.builder.unreachable();
            ctx.builder.const_i32(match bits {
                BitSize::BYTE => 8,
                BitSize::WORD => 16,
                BitSize::DWORD => 32,
                _ => {
                    dbg_assert!(false);
                    0
                },
            });
            ctx.builder.get_local(&address_local);
            gen_call_fn2(ctx.builder, "bug_gen_safe_read_write_page_fault");
        }
        ctx.builder.block_end();
    }
    ctx.builder.block_end();

    ctx.builder.free_local(value_local);
    ctx.builder.free_local(can_use_fast_path_local);
    ctx.builder.free_local(phys_addr_local);
    ctx.builder.free_local(entry_local);
}

#[no_mangle]
pub fn bug_gen_safe_read_write_page_fault(bits: i32, addr: u32) {
    dbg_log!("bug: gen_safe_read_write_page_fault {} {:x}", bits, addr);
    dbg_assert!(false);
}

pub fn gen_set_last_op1(builder: &mut WasmBuilder, source: &WasmLocal) {
    builder.const_i32(global_pointers::LAST_OP1 as i32);
    builder.get_local(&source);
    builder.store_aligned_i32(0);
}

pub fn gen_set_last_op2(builder: &mut WasmBuilder, source: &LocalOrImmedate) {
    builder.const_i32(global_pointers::LAST_OP2 as i32);
    source.gen_get(builder);
    builder.store_aligned_i32(0);
}

pub fn gen_set_last_add_result(builder: &mut WasmBuilder, source: &WasmLocal) {
    builder.const_i32(global_pointers::LAST_ADD_RESULT as i32);
    builder.get_local(&source);
    builder.store_aligned_i32(0);
}

pub fn gen_set_last_result(builder: &mut WasmBuilder, source: &WasmLocal) {
    builder.const_i32(global_pointers::LAST_RESULT as i32);
    builder.get_local(&source);
    builder.store_aligned_i32(0);
}

pub fn gen_set_last_op_size(builder: &mut WasmBuilder, value: i32) {
    builder.const_i32(global_pointers::LAST_OP_SIZE as i32);
    builder.const_i32(value);
    builder.store_aligned_i32(0);
}

pub fn gen_set_flags_changed(builder: &mut WasmBuilder, value: i32) {
    builder.const_i32(global_pointers::FLAGS_CHANGED as i32);
    builder.const_i32(value);
    builder.store_aligned_i32(0);
}

pub fn gen_set_flags_bits(builder: &mut WasmBuilder, bits_to_set: i32) {
    builder.const_i32(global_pointers::FLAGS as i32);
    builder.load_aligned_i32(global_pointers::FLAGS);
    builder.const_i32(bits_to_set);
    builder.or_i32();
    builder.store_aligned_i32(0);
}

pub fn gen_clear_flags_bits(builder: &mut WasmBuilder, bits_to_clear: i32) {
    builder.const_i32(global_pointers::FLAGS as i32);
    builder.load_aligned_i32(global_pointers::FLAGS);
    builder.const_i32(!bits_to_clear);
    builder.and_i32();
    builder.store_aligned_i32(0);
}

pub fn gen_getzf(builder: &mut WasmBuilder) {
    builder.load_aligned_i32(global_pointers::FLAGS_CHANGED);
    builder.const_i32(FLAG_ZERO);
    builder.and_i32();
    builder.if_i32();

    builder.load_aligned_i32(global_pointers::LAST_RESULT);
    let last_result = builder.tee_new_local();
    builder.const_i32(-1);
    builder.xor_i32();
    builder.get_local(&last_result);
    builder.free_local(last_result);
    builder.const_i32(1);
    builder.sub_i32();
    builder.and_i32();
    builder.load_aligned_i32(global_pointers::LAST_OP_SIZE);
    builder.shr_u_i32();
    builder.const_i32(1);
    builder.and_i32();

    builder.else_();
    builder.load_aligned_i32(global_pointers::FLAGS);
    builder.const_i32(FLAG_ZERO);
    builder.and_i32();
    builder.block_end();
}

pub fn gen_getcf(builder: &mut WasmBuilder) {
    builder.load_aligned_i32(global_pointers::FLAGS_CHANGED);
    builder.const_i32(FLAG_CARRY);
    builder.and_i32();
    builder.if_i32();

    builder.load_aligned_i32(global_pointers::LAST_OP1);
    let last_op1 = builder.tee_new_local();

    builder.load_aligned_i32(global_pointers::LAST_OP2);
    let last_op2 = builder.tee_new_local();

    builder.xor_i32();

    builder.get_local(&last_op2);
    builder.load_aligned_i32(global_pointers::LAST_ADD_RESULT);
    builder.xor_i32();

    builder.and_i32();

    builder.get_local(&last_op1);
    builder.xor_i32();

    builder.free_local(last_op1);
    builder.free_local(last_op2);

    builder.load_aligned_i32(global_pointers::LAST_OP_SIZE);
    builder.shr_u_i32();
    builder.const_i32(1);
    builder.and_i32();

    builder.else_();
    builder.load_aligned_i32(global_pointers::FLAGS);
    builder.const_i32(FLAG_CARRY);
    builder.and_i32();
    builder.block_end();
}

pub fn gen_test_be(builder: &mut WasmBuilder) {
    // TODO: Could be made lazy
    gen_getcf(builder);
    gen_getzf(builder);
    builder.or_i32();
}

pub fn gen_test_loopnz(ctx: &mut JitContext, is_asize_32: bool) {
    gen_test_loop(ctx, is_asize_32);
    ctx.builder.eqz_i32();
    gen_getzf(&mut ctx.builder);
    ctx.builder.or_i32();
    ctx.builder.eqz_i32();
}
pub fn gen_test_loopz(ctx: &mut JitContext, is_asize_32: bool) {
    gen_test_loop(ctx, is_asize_32);
    ctx.builder.eqz_i32();
    gen_getzf(&mut ctx.builder);
    ctx.builder.eqz_i32();
    ctx.builder.or_i32();
    ctx.builder.eqz_i32();
}
pub fn gen_test_loop(ctx: &mut JitContext, is_asize_32: bool) {
    if is_asize_32 {
        gen_get_reg32(ctx, regs::ECX);
    }
    else {
        gen_get_reg16(ctx, regs::CX);
    }
    ctx.builder.const_i32(1);
    ctx.builder.sub_i32();
    if is_asize_32 {
        gen_set_reg32(ctx, regs::ECX);
        gen_get_reg32(ctx, regs::ECX);
    }
    else {
        gen_set_reg16(ctx, regs::ECX);
        gen_get_reg16(ctx, regs::CX);
    }
}
pub fn gen_test_jcxz(ctx: &mut JitContext, is_asize_32: bool) {
    if is_asize_32 {
        gen_get_reg32(ctx, regs::ECX);
    }
    else {
        gen_get_reg16(ctx, regs::CX);
    }
    ctx.builder.eqz_i32();
}

pub fn gen_fpu_get_sti(ctx: &mut JitContext, i: u32) {
    ctx.builder.const_i32(i as i32);
    gen_call_fn1_ret_f64(ctx.builder, "fpu_get_sti");
}

pub fn gen_fpu_load_m32(ctx: &mut JitContext, modrm_byte: u8) {
    gen_modrm_resolve_safe_read32(ctx, modrm_byte);
    ctx.builder.reinterpret_i32_as_f32();
    ctx.builder.promote_f32_to_f64();
}

pub fn gen_fpu_load_m64(ctx: &mut JitContext, modrm_byte: u8) {
    gen_modrm_resolve_safe_read64(ctx, modrm_byte);
    ctx.builder.reinterpret_i64_as_f64();
}

pub fn gen_trigger_ud(ctx: &mut JitContext) {
    gen_move_registers_from_locals_to_memory(ctx);
    gen_set_previous_eip_offset_from_eip_with_low_bits(
        ctx.builder,
        ctx.start_of_current_instruction as i32 & 0xFFF,
    );
    gen_fn0_const(ctx.builder, "trigger_ud");
    gen_debug_track_jit_exit(ctx.builder, ctx.start_of_current_instruction);
    ctx.builder.return_();
}

pub fn gen_trigger_gp(ctx: &mut JitContext, error_code: u32) {
    gen_move_registers_from_locals_to_memory(ctx);
    gen_set_previous_eip_offset_from_eip_with_low_bits(
        ctx.builder,
        ctx.start_of_current_instruction as i32 & 0xFFF,
    );
    gen_fn1_const(ctx.builder, "trigger_gp", error_code);
    gen_debug_track_jit_exit(ctx.builder, ctx.start_of_current_instruction);
    ctx.builder.return_();
}

pub fn gen_condition_fn(ctx: &mut JitContext, mut condition: u8) {
    if condition & 0xF0 == 0x00 || condition & 0xF0 == 0x70 || condition & 0xF0 == 0x80 {
        condition &= 0xF;
        if condition == 2 {
            gen_getcf(ctx.builder);
        }
        else if condition == 3 {
            gen_getcf(ctx.builder);
            ctx.builder.eqz_i32();
        }
        else if condition == 4 {
            gen_getzf(ctx.builder);
        }
        else if condition == 5 {
            gen_getzf(ctx.builder);
            ctx.builder.eqz_i32();
        }
        else if condition == 6 {
            gen_test_be(ctx.builder);
        }
        else if condition == 7 {
            gen_test_be(ctx.builder);
            ctx.builder.eqz_i32();
        }
        else {
            let condition_name = CONDITION_FUNCTIONS[condition as usize];
            gen_fn0_const_ret(ctx.builder, condition_name);
        }
    }
    else {
        // loop, loopnz, loopz, jcxz
        dbg_assert!(condition & !0x3 == 0xE0);
        if condition == 0xE0 {
            gen_test_loopnz(ctx, ctx.cpu.asize_32());
        }
        else if condition == 0xE1 {
            gen_test_loopz(ctx, ctx.cpu.asize_32());
        }
        else if condition == 0xE2 {
            gen_test_loop(ctx, ctx.cpu.asize_32());
        }
        else if condition == 0xE3 {
            gen_test_jcxz(ctx, ctx.cpu.asize_32());
        }
    }
}

const RECORD_LOCAL_MEMORY_MOVES_AT_COMPILE_TIME: bool = false;

pub fn gen_move_registers_from_locals_to_memory(ctx: &mut JitContext) {
    let instruction = ::cpu::read32(ctx.start_of_current_instruction);
    if RECORD_LOCAL_MEMORY_MOVES_AT_COMPILE_TIME {
        ::opstats::record_opstat_unguarded_register(instruction);
    }
    else {
        ::opstats::gen_opstat_unguarded_register(ctx.builder, instruction);
    }

    for i in 0..8 {
        ctx.builder
            .const_i32(global_pointers::get_reg32_offset(i as u32) as i32);
        ctx.builder.get_local(&ctx.register_locals[i]);
        ctx.builder.store_aligned_i32(0);
    }
}
pub fn gen_move_registers_from_memory_to_locals(ctx: &mut JitContext) {
    let instruction = ::cpu::read32(ctx.start_of_current_instruction);
    if RECORD_LOCAL_MEMORY_MOVES_AT_COMPILE_TIME {
        ::opstats::record_opstat_unguarded_register(instruction);
    }
    else {
        ::opstats::gen_opstat_unguarded_register(ctx.builder, instruction);
    }

    for i in 0..8 {
        ctx.builder
            .const_i32(global_pointers::get_reg32_offset(i as u32) as i32);
        ctx.builder.load_aligned_i32_from_stack(0);
        ctx.builder.set_local(&ctx.register_locals[i]);
    }
}

pub fn gen_profiler_stat_increment(builder: &mut WasmBuilder, stat: profiler::stat) {
    if !cfg!(feature = "profiler") || !cfg!(feature = "profiler_instrument") {
        return;
    }
    let addr = unsafe { profiler::stat_array.as_mut_ptr().offset(stat as isize) } as u32;
    gen_increment_variable(builder, addr, 1)
}

pub fn gen_debug_track_jit_exit(builder: &mut WasmBuilder, address: u32) {
    if cfg!(feature = "profiler") && cfg!(feature = "profiler_instrument") {
        gen_fn1_const(builder, "track_jit_exit", address);
    }
}
