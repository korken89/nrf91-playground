#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use defmt::{error, info, unwrap};
use embassy_executor::Spawner;
use embassy_nrf::gpio::{Flex, Level, Output, OutputDrive};
use embassy_nrf::interrupt::{self, InterruptExt, Priority};
use embassy_nrf::pac::{UARTE0, UARTE1};
use embassy_nrf::pwm::{Prescaler, SimplePwm};
use embassy_nrf::saadc::{ChannelConfig, Config, Saadc};
use embassy_time::{with_timeout, Duration, Ticker, Timer};
use futures::StreamExt;
use nrf_modem::{ConnectionPreference, DtlsSocket, PeerVerification, SystemMode, TcpStream};
use propane_monitor_embassy::psk::install_psk_id_and_psk;
use propane_monitor_embassy::*;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    // Set up the interrupts for the modem
    let egu1 = interrupt::take!(EGU1);
    egu1.set_priority(Priority::P4);
    egu1.set_handler(|_| {
        nrf_modem::application_irq_handler();
        cortex_m::asm::sev();
    });
    egu1.enable();

    let ipc = interrupt::take!(IPC);
    ipc.set_priority(Priority::P0);
    ipc.set_handler(|_| {
        nrf_modem::ipc_irq_handler();
        cortex_m::asm::sev();
    });
    ipc.enable();

    // // Disable UARTE for lower power consumption
    let uarte0: UARTE0 = unsafe { core::mem::transmute(()) };
    let uarte1: UARTE1 = unsafe { core::mem::transmute(()) };
    uarte0.enable.write(|w| w.enable().disabled());
    uarte1.enable.write(|w| w.enable().disabled());

    // Initialize heap data
    alloc_init();

    // Run our sampling program, will not return unless an error occurs
    match run().await {
        Ok(()) => unreachable!(),
        Err(e) => {
            // If we get here, we have problems
            error!("app exited: {:?}", defmt::Debug2Format(&e));
            exit();
        }
    }
}

async fn run() -> Result<(), Error> {
    // Handle for device peripherals
    let p = embassy_nrf::init(Default::default());

    // Icarus: Has an eSIM and an External SIM.  Use Pin 8 to select: HIGH = eSIM, Low = External
    // Only change SIM selection while modem is off (AT+CFUN=1)
    // let _sim_select = Output::new(p.P0_08, Level::Low, OutputDrive::Standard);

    // Stratus: Pin 25 to control VBAT_MEAS_EN, Power must connect to V_Bat to measure correctly
    // Icarus: Pin 07 to disable battery charging circuit
    // let mut enable_bat_meas = Output::new(p.P0_25, Level::Low, OutputDrive::Standard);
    // let _disable_charging = Output::new(p.P0_07, Level::High, OutputDrive::Standard);

    // Stratus: Pin 3 for blue LED power when data is being transmitted
    // Stratus: Pin 12 for blue LED power when data is being transmitted, (red: P_10, green: P_11)
    let mut led = Output::new(p.P0_03, Level::High, OutputDrive::Standard);

    // Initialize cellular modem
    unwrap!(
        nrf_modem::init(SystemMode {
            lte_support: true,
            lte_psm_support: true,
            nbiot_support: false,
            gnss_support: false,
            preference: ConnectionPreference::Lte,
        })
        .await
    );

    // unwrap!(nrf_modem::send_at::<128>("AT+CGDCONT=0,\"IP\",\"iot.1nce.net\"").await);

    // Configure GPS settings
    // config_gnss().await?;

    // install PSK info for secure cloud connectivity
    install_psk_id_and_psk().await?;

    // Heapless buffer to hold our sample values before transmitting
    let mut payload = Payload::new();

    use nrf_modem::no_std_net::*;
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(79, 136, 27, 216)), 5684);
    info!("TCP open");
    let s = TcpStream::connect(addr).await?;
    info!("TCP opened");

    // Create our sleep timer (time between sensor measurements)
    let mut ticker = Ticker::every(Duration::from_secs(5));
    info!("Entering Loop");

    loop {
        // payload
        //     .data
        //     .push(TankLevel::new(
        //         convert_to_tank_level(10000),
        //         1987,
        //         convert_to_mv(1000),
        //     ))
        //     .unwrap();

        // Visibly show that data is being sent
        led.set_low();

        s.write(b"Hello from nRF91!\n").await?;
        let sig_strength = rssi().await?;
        info!("Signal Strength: {} dBm", sig_strength);

        // If timeout occurs, log a timeout and continue.
        // if let Ok(_) = with_timeout(Duration::from_secs(180), transmit_payload(&mut payload)).await
        // {
        //     payload.timeouts = 0;

        //     info!("Transfer Complete");
        // } else {
        //     payload.timeouts += 1;
        //     info!(
        //         "Timeout has occurred {} time(s), data clear and start over",
        //         payload.timeouts
        //     );
        // }

        // payload.data.clear();

        led.set_high();

        info!("Ticker next()");
        ticker.next().await; // wait for next tick event
    }
}
