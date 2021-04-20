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
    let mut attempts = 0;
    while attempts < 10 {
        match client() {
            Ok(_) => break,
            Err(e) => {
                eprintln!("client() -> {}", e);
                attempts += 1;
                // todo: add an attempts reset after a certain amount of time passes
                std::thread::sleep(std::time::Duration::from_secs(6));
            }
        }
    }
    eprintln!("main() -> client returned ok or attempts > 10");
    Ok(())
}

fn client() -> Result<(), Box<dyn std::error::Error>> {
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

    let mut auth_attempts: u8 = 0;
    while auth_attempts < 10 {
        let seen_by: u64 = redis::cmd("publish").arg("clients:msg:connecting").arg(&msg_string).query(&mut rc)?;
        if seen_by > 0 { break }
        println!("main() -> request failed, no handlers on server");
        auth_attempts += 1;
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    println!("main() -> request sent, waiting for auth");
    let has_auth: bool = loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        match redis::cmd("auth").arg(["client:", &u].concat()).arg(&p).query(&mut rc) {
            Ok(o) => break o,
            Err(_) => {
                auth_attempts += 1;
                if auth_attempts > 10 { break false };
            },
        };
    };

    if has_auth {
        println!("main() -> auth granted, connection successful, listening for messages");
        let mut sub = rc.as_pubsub();
        sub.subscribe("clients:msg:all")?;
        sub.subscribe(["clients:msg:", &u].concat())?;

        loop {
            let msg_string: String = sub.get_message()?.get_payload()?;
            tokio::task::spawn(async{ if let Err(e) = msg_handler(msg_string).await {eprintln!("msg_handler() -> {}", e)} });
        }
    } else {
        println!("main() -> auth failed, timed out");
    }
    
    Ok(())
}

async fn msg_handler(msg_string: String) -> Result<(), Box<dyn std::error::Error>> {
    println!("msg_handler -> {}", msg_string);
    Ok(())
}
