/* STM32N6570-DK, RAM-only load via probe-rs ("dev mode").
 *
 * THE IMPORTANT PART — this window, not 0x34000000.
 *
 * In boot-ROM dev mode only AXISRAM2's top half is usable by the Reset handler:
 *
 *   [0x34180000, 0x34200000)   512 KB, boot-ROM-safe
 *   below 0x34180000           NOT CPU-accessible yet -> hard-faults at reset
 *                              with PC=0, before the vector table even runs
 *   above 0x34200000           not clocked yet (RCC.memenr axisram3..6en)
 *
 * That's why an image linked at 0x34000000 is completely silent. Taken from
 * the embassy N6 examples:
 *   examples/stm32n6/memory.x            (this window, code in RAM)
 *   examples/stm32n6-flashboot/          (FSBL + app in XSPI flash)
 *
 * FLASH and RAM must not alias: cortex-m-rt puts the vector table + .text in
 * FLASH and .data/.bss in RAM, with .data's load address at the end of FLASH.
 */
MEMORY
{
  FLASH : ORIGIN = 0x34180000, LENGTH = 256K
  RAM   : ORIGIN = 0x341C0000, LENGTH = 256K
}
