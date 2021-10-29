use std::{thread, time, io};
use std::fs::{File, OpenOptions};
use std::io::{Write, Read, prelude::*, SeekFrom};
use serde::{Serialize, Deserialize};
use serde_json::json;
use serialport::SerialPortBuilder;

const STR_START_SIGN : &str = "START_UPD^_^";
const STR_RECIVE_SUCCESS : &str="RECEIVESTART";

#[derive(Debug, PartialEq, Eq)]
enum UpdateStep {
    UpdateSendStartSign,
    UpdateRecvSuccess,
    UpdateDone,
}

#[repr(C)]
#[derive(Default, Debug, Serialize, Deserialize)]
struct RxCmd {
    sign:       u16,
    cmd:        u8,
    status:     u8,
    addr:       u32,
    len:        u32, // This field is the length of the data when the command is received.
    crc:        u16,
    reserve:    u16,
}

#[repr(C)]
#[derive(Default, Debug, Serialize, Deserialize)]
struct TxCmd {
    sign:       u16,
    cmd:        u8,
    status:     u8,
    addr:       u32,
    data_crc:   u32, // This field is the crc of the data when the command is transmitted.
    crc:        u16,
    reserve:    u16,
}

#[repr(C)]
struct UpdateMaster {
    port: SerialPortBuilder,
    rx_buf: Vec<u8>,
    step: UpdateStep,
    rxcmd: RxCmd,
    txcmd: TxCmd,
    addr: u32,
}

impl UpdateMaster {
    fn new(path: &str, baud_rate: u32) -> UpdateMaster {
        let port = serialport::new(path, baud_rate);
        UpdateMaster {
            port,
            rx_buf: vec![0; 512],
            step: UpdateStep::UpdateSendStartSign,
            rxcmd: Default::default(),
            txcmd: Default::default(),
            addr: 0
        }
    }

    fn recv_cmd(&mut self, port: &mut Box<dyn serialport::SerialPort>) {
        loop {
            if self.step != UpdateStep::UpdateDone {
                let n = port.read(&mut self.rx_buf[..]);
                let serial_buf = &self.rx_buf;

                let len = match n {
                    Ok(n) => n,
                    Err(_) => {
                        print!(".");
                        continue
                    },
                };

                println!("{} {:?}", len, &serial_buf[..len]);

                for (i, _) in (&serial_buf[..(len-1)]).iter().enumerate() {
                    if serial_buf[i] == 0xAA && serial_buf[i+1] == 0x55 {
                        // TODO if i + 16 > len
                        self.rxcmd = bincode::deserialize(&serial_buf[i..len]).unwrap();
                        println!("{:?}", self.rxcmd);
                        return;
                    }
                }
            }
            thread::sleep(time::Duration::from_millis(100));
        }
    }
}

fn updater_calc_crc(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for i in 0..data.len() {
        sum += data[i] as u32;
    }
    sum
}

#[derive(Serialize, Deserialize, Debug)]
struct UpdateConfig {
    #[serde(default = "default_update_path")]
    path:           String,
    #[serde(default = "default_update_serialport")]
    serialport:     String,
    #[serde(default = "default_update_baud_rate")]
    baud_rate:      u32,
}

const CONFIG_FILE: &'static str = "updater.json";

fn default_update_path() -> String {
    "app.upd".to_string()
}

#[cfg(windows)]
fn default_update_serialport() -> String {
    "COM1".to_string()
}

#[cfg(unix)]
fn default_update_serialport() -> String {
    "/dev/ttyUSB0".to_string()
}

fn default_update_baud_rate() -> u32 {
    115200
}

fn create_default_config_file() {
    #[cfg(windows)]
    let config = json!({
        "path": "app.upd",
        "serialport": "COM1",
        "baud_rate": 115200
    });

    #[cfg(unix)]
    let config = json!({
        "path": "app.upd",
        "serialport": "/dev/ttyUSB0"
        "baud_rate": 115200
    });

    // TODO: optimize code
    // #[cfg(unix)]
    // {
    //     config["serialport"] = "/dev/ttyUSB0".to_string();
    // }

    let file = OpenOptions::new().write(true).create_new(true).open(CONFIG_FILE);

    match file {
        Ok(mut file) => {
            file.write_all(config.to_string().as_bytes()).expect("write default config failed!");
        },
        _ => ()
    }
}

fn get_config_from_file() -> UpdateConfig {
    let mut file = File::open(CONFIG_FILE).unwrap();
    let mut config_str = String::new();
    file.read_to_string(&mut config_str).unwrap();

    let config: UpdateConfig = serde_json::from_str(&config_str).unwrap();
    config
}

fn main() -> io::Result<()> {
    create_default_config_file();
    let config = get_config_from_file();

    println!("try to open {}", config.serialport);
    let mut updater = UpdateMaster::new(&config.serialport, config.baud_rate);
    let mut port = serialport::new(&config.serialport, config.baud_rate).timeout(time::Duration::from_millis(10)).open().expect("open COM12 failed!");
    println!("open {} successfully", config.serialport);

    // enter update
    loop {
        port.write(STR_START_SIGN.as_bytes()).unwrap();

        let n = port.read(updater.rx_buf.as_mut_slice());
        let serial_buf = &updater.rx_buf;

        let get_str = match n {
            Ok(n) => {
                std::str::from_utf8(&serial_buf[..n]).unwrap()
            },
            Err(_) => {
                print!(".");
                ""
            }
        };

        println!("{}", get_str);
        if get_str == STR_RECIVE_SUCCESS {
            updater.step = UpdateStep::UpdateRecvSuccess;
            break;
        }
        thread::sleep(time::Duration::from_millis(100));
    }

    // update loop
    while updater.step != UpdateStep::UpdateDone {
        thread::sleep(time::Duration::from_millis(50)); // don't reduce the sleep time!

        updater.recv_cmd(&mut port);

        match updater.rxcmd.cmd {
            // check update
            1 => {
                let mut txcmd = &mut updater.txcmd;
                txcmd.sign = 0x55AA;
                txcmd.cmd = 0x1;
                txcmd.addr = 0;
                txcmd.data_crc = 0;

                let txcmd_s = bincode::serialize(&txcmd).unwrap();
                txcmd.crc = updater_calc_crc(&txcmd_s[..12]) as u16;
                println!("{:?}", txcmd);

                port.write(&txcmd_s).unwrap();
            },

            // send data
            2 => {
                let mut file = File::open(&config.path).expect("open update file failed");
                file.seek(SeekFrom::Start(updater.rxcmd.addr as u64)).expect("update file seek failed");
                file.read(&mut updater.rx_buf[..]).expect("update file read failed");

                let rxcmd = &updater.rxcmd;
                let mut txcmd = &mut updater.txcmd;
                txcmd.sign = 0x55AA;
                txcmd.cmd = 0x2;
                txcmd.addr = rxcmd.addr + 512;
                txcmd.data_crc = updater_calc_crc(&updater.rx_buf[..]);

                let txcmd_s = bincode::serialize(&txcmd).unwrap();
                txcmd.crc = updater_calc_crc(&txcmd_s[..12]) as u16;
                println!("{:?}", txcmd);

                // for (i, x) in updater.rx_buf.iter().enumerate() {
                //     if (i+1) % 16 != 0 {
                //         print!("{:02x} ", x);
                //     } else {
                //         println!("{:02x} ", x);
                //     }
                // }

                let txcmd_s = bincode::serialize(&txcmd).unwrap();
                port.write(&txcmd_s).unwrap();
                port.write(&updater.rx_buf[..]).unwrap();
            },

            // read update status
            3 => {
                if updater.rxcmd.status == 0xff {
                    updater.step = UpdateStep::UpdateDone;
                }
            },

            // ignore
            _ => ()
        }
    }
    Ok(())
}
