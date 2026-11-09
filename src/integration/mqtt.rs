use super::{Feed, MqttOptions};
use rumqttc::{AsyncClient, Event, MqttOptions as ClientOptions, Packet, QoS};
use std::{sync::atomic::Ordering::Relaxed, time::Duration};

pub(super) fn publish(feed: &Feed, settings: MqttOptions) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        feed.mqtt_errors.fetch_add(1, Relaxed);
        return;
    };
    let mut random = [0; 8];
    if getrandom::fill(&mut random).is_err() {
        feed.mqtt_errors.fetch_add(1, Relaxed);
        return;
    }
    let id = format!("eipwb-{:016x}", u64::from_le_bytes(random));
    let options = ClientOptions::new(id, settings.broker, settings.port);
    let (client, mut connection) = AsyncClient::new(options, 4);
    runtime.block_on(async {
        let driver = async {
            loop {
                match connection.poll().await {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        feed.mqtt_connected.store(true, Relaxed)
                    }
                    Ok(_) => {}
                    Err(_) => {
                        feed.mqtt_connected.store(false, Relaxed);
                        feed.mqtt_errors.fetch_add(1, Relaxed);
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        };
        let publisher = async {
            let mut poll = tokio::time::interval(Duration::from_millis(settings.period_ms.min(50)));
            poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let period = Duration::from_millis(settings.period_ms);
            let mut next_publish = tokio::time::Instant::now();
            let mut last_index = 0;
            loop {
                poll.tick().await;
                if feed.stop.load(Relaxed) {
                    break;
                }
                let now = tokio::time::Instant::now();
                if now >= next_publish
                    && feed.mqtt_connected.load(Relaxed)
                    && let Some(frame) = feed.latest()
                    && frame.index != last_index
                    && frame.generation == feed.active_generation.load(Relaxed)
                {
                    let payload = feed.frame_json(frame).to_string();
                    if client
                        .try_publish(&settings.topic, QoS::AtMostOnce, false, payload)
                        .is_ok()
                    {
                        last_index = frame.index;
                    } else {
                        feed.mqtt_errors.fetch_add(1, Relaxed);
                    }
                    next_publish = now + period;
                }
            }
        };
        tokio::select! { _ = driver => {}, _ = publisher => {} }
    });
    feed.mqtt_connected.store(false, Relaxed);
}
