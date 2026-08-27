#if defined(__x86_64__)
  .text
  .globl _start
  .hidden _start
  .type _start,@function
_start:
  xor %rbp, %rbp
  call main
  mov %eax, %edi
  call __ck_platform_exit

  .globl __ck_platform_write
  .hidden __ck_platform_write
  .type __ck_platform_write,@function
__ck_platform_write:
  push %r12
  push %r13
  push %r14
  sub $32, %rsp
  mov %rdi, %r12
  mov %rsi, %r13
  mov %rdx, %r14
  movq $1, 0(%rsp)
  movq $0, 8(%rsp)
  movq $0, 16(%rsp)
  movq $0, 24(%rsp)
  mov $13, %edi
  mov %rsp, %rsi
  xor %edx, %edx
  mov $8, %r10d
  mov $13, %eax
  syscall
  mov %r12, %rdi
  mov %r13, %rsi
  mov %r14, %rdx
  mov $1, %rax
  syscall
  add $32, %rsp
  pop %r14
  pop %r13
  pop %r12
  ret

  .globl __ck_platform_exit
  .hidden __ck_platform_exit
  .type __ck_platform_exit,@function
__ck_platform_exit:
  mov $60, %rax
  syscall
  ud2
#elif defined(__aarch64__)
  .text
  .globl _start
  .hidden _start
  .type _start,%function
_start:
  bl main
  bl __ck_platform_exit

  .globl __ck_platform_write
  .hidden __ck_platform_write
  .type __ck_platform_write,%function
__ck_platform_write:
  stp x19, x20, [sp, #-64]!
  str x21, [sp, #16]
  mov x19, x0
  mov x20, x1
  mov x21, x2
  mov x4, #1
  str x4, [sp, #32]
  str xzr, [sp, #40]
  str xzr, [sp, #48]
  str xzr, [sp, #56]
  mov x0, #13
  add x1, sp, #32
  mov x2, #0
  mov x3, #8
  mov x8, #134
  svc #0
  mov x0, x19
  mov x1, x20
  mov x2, x21
  mov x8, #64
  svc #0
  ldr x21, [sp, #16]
  ldp x19, x20, [sp], #64
  ret

  .globl __ck_platform_exit
  .hidden __ck_platform_exit
  .type __ck_platform_exit,%function
__ck_platform_exit:
  mov x8, #93
  svc #0
  brk #0
#else
#error unsupported Linux runtime architecture
#endif
