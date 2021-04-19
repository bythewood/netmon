#[derive(serde_derive::Serialize, serde_derive::Deserialize, std::default::Default)]
struct Msg {
    stamp: i64,
    from: String,
    event: String,
    action: String,    
    target: String,
    data: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rcl: redis::Client = redis::Client::open(redis::ConnectionInfo{
        addr: Box::new(redis::ConnectionAddr::TcpTls{
            host: std::env::var("NETMON_SERVER").unwrap(),
            port: 10101,
            insecure: false,
        }),
        db: 0,
        username: Option::Some("public".to_owned()),
        passwd: Option::Some("public".to_owned()),
    })?;
    let mut rc = rcl.get_connection()?;

    let u: String = redis::cmd("acl").arg("genpass").arg("1024").query(&mut rc)?;
    let p: String = redis::cmd("acl").arg("genpass").arg("1024").query(&mut rc)?;
    let msg = Msg {
        from: u.to_owned(),
        action: "auth_request".to_owned(),
        data: p.to_owned(),
        .. Msg::default()
    };
    let msg_string: String = toml::to_string(&msg)?;
    loop {
        let seen_by: u64 = redis::cmd("publish").arg("clients:msg:connecting").arg(&msg_string).query(&mut rc)?;
        if seen_by > 0 { break }
        println!("main() -> request failed, no handlers on server, waiting 10s");
        std::thread::sleep(std::time::Duration::from_secs(10));
    }

    println!("main() -> request sent, waiting for auth");

    let mut auth_attempts: u8 = 0;
    let has_auth: bool = loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        match redis::cmd("auth").arg(["client:", &u].concat()).arg(&p).query(&mut rc) {
            Ok(o) => break o,
            Err(_) => {
                auth_attempts += 1;
                if auth_attempts > 30 { break false };
            },
        };
    };

    if has_auth {
        println!("main() -> auth granted, connection successful, listening for messages");
        let mut sub = rc.as_pubsub();
        sub.subscribe("clients:msg:all")?;
        sub.subscribe(["clients:msg:", &u].concat())?;

        loop {
            let msg = sub.get_message()?;
            let text: String = msg.get_payload()?;
            println!("main() -> msg seen: {}", text);
        }
    } else {
        println!("main() -> auth failed, timed out")
    }
    
    Ok(())
}
