//! Reenvia ao aparelho o estado que ele já tem (não muda nada) e confere a resposta.
use ar_midea::config::Config;
use ar_midea::device::Device;

fn main() {
    let cfg = Config::load().expect("config.json");
    let addr = format!("{}:{}", cfg.host, cfg.port).parse().unwrap();
    let mut dev = Device::new(addr, cfg.device_id, &cfg.token, &cfg.key).unwrap();
    dev.authenticate().unwrap();
    dev.get_capabilities().unwrap();
    dev.refresh().unwrap();
    let before = dev.state.clone();
    println!("antes: power={} modo={:?} alvo={} ventilador={} oscilação={:?}", before.power, before.mode, before.target_temperature, before.fan_speed, before.swing);
    let t = std::time::Instant::now();
    dev.apply(&before).expect("apply");
    println!("SetState respondido em {:?}", t.elapsed());
    let after = &dev.state;
    println!("depois: power={} modo={:?} alvo={} ventilador={} oscilação={:?}", after.power, after.mode, after.target_temperature, after.fan_speed, after.swing);
    assert_eq!((before.power, before.mode, before.target_temperature, before.swing), (after.power, after.mode, after.target_temperature, after.swing));
    println!("ok: estado inalterado");
}
