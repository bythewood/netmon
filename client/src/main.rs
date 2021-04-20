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
        let start = chrono::Utc::now().timestamp();
        match client() {
            Ok(_) => (),
            Err(e) => {
                if chrono::Utc::now().timestamp() - start > 60 { attempts = 0 }
                eprintln!("client() -> {}", e);
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_secs(6));
            }
        }
    }
    eprintln!("main() -> client errors exceeded max allowed in short order");
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
        stamp: chrono::Utc::now().timestamp(),
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
        auth_attempts += 1;
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    while auth_attempts < 10 {
        std::thread::sleep(std::time::Duration::from_secs(1));
        match redis::cmd("auth").arg(["client:", &u].concat()).arg(&p).query(&mut rc) {
            Ok(o) => o,
            Err(_) => {
                auth_attempts += 1;
            },
        };
    }

    let mut sub = rc.as_pubsub();
    sub.subscribe("clients:msg:all")?;
    sub.subscribe(["clients:msg:", &u].concat())?;

    loop {
        let u = u.clone();
        let p = p.clone();
        let msg_string: String = sub.get_message()?.get_payload()?;
        tokio::task::spawn(async{
            if let Err(e) = msg_handler(u, p, msg_string).await {
                eprintln!("msg_handler() -> {}", e);
            }
        });
    }
}

async fn msg_handler(u: String, p: String, msg_string: String) -> Result<(), Box<dyn std::error::Error>> {
    let rcl: redis::Client = redis::Client::open(redis::ConnectionInfo{
        addr: Box::new(redis::ConnectionAddr::TcpTls{
            host: std::env::var("NETMON_SERVER").unwrap(),
            port: 10101,
            insecure: false,
        }),
        db: 0,
        username: Option::Some(u.clone()),
        passwd: Option::Some(p),
    })?;
    let msg: Msg = toml::from_str(&msg_string)?;
    if msg.from == "server" && msg.event == "query" && msg.target == "clients" {
        if msg.action == "env_vars_os" {
            let mut env_vars_os_map = std::collections::HashMap::new();
            for (key, val) in std::env::vars_os() {
                if let (Ok(k), Ok(v)) = (key.into_string(), val.into_string()) {
                    env_vars_os_map.insert(k, v);
                }
            }
            let env_vars_os_string = toml::to_string(&env_vars_os_map)?;
            let msg = Msg {
                stamp: chrono::Utc::now().timestamp(),
                from: u.clone(),
                target: "server".to_owned(),
                event: "query".to_owned(),
                action: "env_vars_os".to_owned(),
                data: env_vars_os_string,
            };
            let msg_string = toml::to_string(&msg)?;
            let mut rc = rcl.get_tokio_connection().await?;
            redis::cmd("publish").arg(["clients:msg:", &u.clone()].concat()).arg(msg_string).query_async(&mut rc).await?;
        }
    }
    Ok(())
}
