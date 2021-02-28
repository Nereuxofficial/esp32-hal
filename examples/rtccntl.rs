#![no_std]
#![no_main]

use core::fmt::Write;
use core::panic::PanicInfo;

use esp32_hal::prelude::*;

use esp32_hal::clock_control::{sleep, CPUSource, ClockControl};
use esp32_hal::dport::Split;
use esp32_hal::dprintln;
use esp32_hal::serial::{config::Config, Serial};
use esp32_hal::target;

const BLINK_HZ: Hertz = Hertz(1);

#[no_mangle]
fn main() -> ! {
    let dp = target::Peripherals::take().expect("Failed to obtain Peripherals");

    let mut timg0 = dp.TIMG0;
    let mut timg1 = dp.TIMG1;

    // (https://github.com/espressif/openocd-esp32/blob/97ba3a6bb9eaa898d91df923bbedddfeaaaf28c9/src/target/esp32.c#L431)
    // openocd disables the watchdog timers on halt
    // we will do it manually on startup
    disable_timg_wdts(&mut timg0, &mut timg1);

    let (_, dport_clock_control) = dp.DPORT.split();

    // setup clocks & watchdog
    let mut clock_control = ClockControl::new(
        dp.RTCCNTL,
        dp.APB_CTRL,
        dport_clock_control,
        esp32_hal::clock_control::XTAL_FREQUENCY_AUTO,
    )
    .unwrap();

    // set desired clock frequencies
    clock_control
        .set_cpu_frequencies(
            CPUSource::Xtal,
            10.MHz(),
            CPUSource::Xtal,
            240.MHz(),
            CPUSource::PLL,
            80.MHz(),
        )
        .unwrap();

    let (clock_control_config, mut watchdog) = clock_control.freeze().unwrap();

    watchdog.start(3.s());

    let gpios = dp.GPIO.split();
    // setup serial controller
    let mut uart0: Serial<_, _, _> = Serial::new(
        dp.UART0,
        esp32_hal::serial::Pins {
            tx: gpios.gpio1,
            rx: gpios.gpio3,
            cts: None,
            rts: None,
        },
        Config::default(),
        clock_control_config,
    )
    .unwrap();
    uart0.change_baudrate(115200).unwrap();

    // print startup message
    writeln!(uart0, "\n\nReboot!\n",).unwrap();

    writeln!(uart0, "Running on core {:?}\n", esp32_hal::get_core()).unwrap();

    writeln!(
        uart0,
        "UART0 baudrate: {}, using apb clock instead of ref clock: {}\n",
        uart0.baudrate(),
        uart0.is_clock_apb()
    )
    .unwrap();

    writeln!(uart0, "{:?}\n", clock_control_config).unwrap();
    writeln!(uart0, "{:?}\n", watchdog.config().unwrap()).unwrap();

    // register callback which is called when the clock is switched
    clock_control_config
        .add_callback(&|before_source,
                        before_freq,
                        before_apb_freq,
                        after_source,
                        after_freq,
                        after_apb_freq| {
            dprintln!(
                "  Before: Source: {:?}, CPU: {}, APB: {},    After: Source: {:?}, CPU: {}, APB: {}",
                before_source,
                before_freq,
                before_apb_freq,
                after_source,
                after_freq,
                after_apb_freq
            );
        })
        .unwrap();

    // uncomment next line to test panic exit
    // panic!("panic test");

    // main loop, which in turn lock and unlocks apb and cpu locks
    let mut x: u32 = 0;
    let mut prev_ccount = 0;
    loop {
        for j in 0..2 {
            let apb_guard = if j == 1 {
                Some(clock_control_config.lock_apb_frequency())
            } else {
                None
            };

            for i in 0..2 {
                let cpu_guard = if i == 1 {
                    Some(clock_control_config.lock_cpu_frequency())
                } else {
                    None
                };

                x = x.wrapping_add(1);

                let ccount = xtensa_lx::timer::get_cycle_count();
                let ccount_diff = ccount.wrapping_sub(prev_ccount);

                writeln!(
                    uart0,
                    "Loop: {}, cycles: {}, cycles since previous {}",
                    x, ccount, ccount_diff
                )
                .unwrap();

                prev_ccount = ccount;

                sleep((Hertz(1_000_000) / BLINK_HZ).us());

                // comment out next line to check watchdog behavior
                watchdog.feed();

                if cpu_guard.is_some() {
                    drop(cpu_guard.unwrap())
                }
            }
            if apb_guard.is_some() {
                drop(apb_guard.unwrap())
            }
        }
    }
}

const WDT_WKEY_VALUE: u32 = 0x50D83AA1;

fn disable_timg_wdts(timg0: &mut target::TIMG0, timg1: &mut target::TIMG1) {
    timg0
        .wdtwprotect
        .write(|w| unsafe { w.bits(WDT_WKEY_VALUE) });
    timg1
        .wdtwprotect
        .write(|w| unsafe { w.bits(WDT_WKEY_VALUE) });

    timg0.wdtconfig0.write(|w| unsafe { w.bits(0x0) });
    timg1.wdtconfig0.write(|w| unsafe { w.bits(0x0) });
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    dprintln!("\n\n*** {:?}", info);
    loop {}
}
