#[derive(serde_derive::Deserialize, std::default::Default)]
struct Auth {
    username: String,
    password: String,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, std::default::Default)]
struct Msg {
    stamp: i64,
    from: String,
    event: String,
    action: String,    
    target: String,
    data: String,
}

lazy_static::lazy_static!{
    static ref REDIS_CLIENT: redis::Client = {
        let auth: Auth = toml::from_str(std::fs::read_to_string("./auth.toml").unwrap_or("".to_owned()).as_str()).unwrap_or_default();
        redis::Client::open(redis::ConnectionInfo{
            addr: Box::new(redis::ConnectionAddr::Unix(std::path::PathBuf::from("/tmp/redis.sock"))),
            db: 0,
            username: Option::Some(auth.username),
            passwd: Option::Some(auth.password),
        }).unwrap()
    };
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut rc = REDIS_CLIENT.get_tokio_connection().await?;

    redis::cmd("set").arg("heartbeat").arg(chrono::Utc::now().timestamp()).query_async(&mut rc).await?;

    tokio::task::spawn(async{ if let Err(e) = heartbeat().await {println!("heartbeat() -> {}", e)} });
    tokio::task::spawn(async{ if let Err(e) = client_handler().await {println!("client_handler() -> {}", e)} });
    tokio::task::yield_now().await;

    tokio::task::spawn(async{ if let Err(e) = test_client().await {println!("test_client() -> {}", e)} });

    loop {
        let heartbeat: i64 = redis::cmd("get").arg("heartbeat").query_async(&mut rc).await.unwrap_or(0);
        let timestamp = chrono::Utc::now().timestamp();

        let diff = timestamp - heartbeat;

        if diff > 60 {
            println!("Heartbeat is behind by {} seconds. Assuming death and ending process.", diff);
            std::process::exit(1);
        }

        std::thread::sleep(std::time::Duration::from_secs(10));
    }
}

async fn heartbeat() -> Result<(), Box<dyn std::error::Error>> {
    let mut rc = REDIS_CLIENT.get_tokio_connection().await?;
    loop {
        redis::cmd("set").arg("heartbeat").arg(chrono::Utc::now().timestamp()).query_async(&mut rc).await?;
        let mut msg = Msg::default();
        msg.stamp = chrono::Utc::now().timestamp();
        msg.from = "server".to_owned();
        msg.event = "heartbeat".to_owned();
        redis::cmd("publish").arg("clients:msg:all").arg(toml::to_string(&msg)?).query_async(&mut rc).await?;

        redis::cmd("setnx").arg("reset").arg(true).query_async(&mut rc).await?;
        let reset: bool = redis::cmd("get").arg("reset").query_async(&mut rc).await?;
        if reset {
            println!("Database reset invoked.");
            redis::cmd("flushdb").query_async(&mut rc).await?;
            redis::cmd("set").arg("reset").arg(false).query_async(&mut rc).await?;
            redis::cmd("acl").arg("load").query_async(&mut rc).await?;
            redis::cmd("set").arg("audit").arg(false).query_async(&mut rc).await?;
        }

        redis::cmd("setnx").arg("audit").arg(true).query_async(&mut rc).await?;
        let audit: bool = redis::cmd("get").arg("audit").query_async(&mut rc).await?;
        if audit {
            println!("Client audit invoked.");
            redis::cmd("set").arg("audit").arg(false).query_async(&mut rc).await?;
            // do audit of dead client records and acls
        }

        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}

async fn client_handler() -> Result<(), Box<dyn std::error::Error>> {
    let mut rc = REDIS_CLIENT.get_tokio_connection().await?;

    let mut sub_rc = REDIS_CLIENT.get_connection()?;
    let mut sub = sub_rc.as_pubsub();

    sub.subscribe("handlers:msg:all")?;
    sub.subscribe("clients:msg:connecting")?;

    loop {
        let msg_string: String = sub.get_message()?.get_payload()?;
        let msg: Msg = toml::from_str(&msg_string)?;

        if msg.action == "auth_request" {
            if msg.from.len() == 0 || msg.data.len() == 0 { continue };
            redis::cmd("sadd").arg("clients").arg(&msg.from).query_async(&mut rc).await?;
            redis::cmd("acl")
                .arg("setuser")
                .arg(["client:", &msg.from].concat())
                .arg("on")
                .arg("sanitize-payload")
                .arg([">", &msg.data].concat())
                .arg("nocommands")
                .arg(["+publish|clients:msg:", &msg.from].concat())
                .arg(["+subscribe|clients:msg:", &msg.from].concat())
                .arg("+subscribe|clients:msg:all")
                .arg("resetchannels")
                .arg("&clients:msg:all")
                .arg(["&clients:msg:", &msg.from].concat())
            .query_async(&mut rc).await?;
        }
    }
}

async fn test_client() -> Result<(), Box<dyn std::error::Error>> {
    let rcl: redis::Client = redis::Client::open(redis::ConnectionInfo{
        addr: Box::new(redis::ConnectionAddr::TcpTls{
            host: "niflheimgames.com".to_owned(),
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
    let mut msg = Msg::default();
    msg.from = u.to_owned();
    msg.action = "auth_request".to_owned();
    msg.data = p.to_owned();
    let msg_string: String = toml::to_string(&msg)?;
    loop {
        let seen_by: u64 = redis::cmd("publish").arg("clients:msg:connecting").arg(&msg_string).query(&mut rc)?;
        if seen_by > 0 { break }
        println!("test_client() -> request failed, no handlers on server, waiting 10s");
        std::thread::sleep(std::time::Duration::from_secs(10));
    }

    println!("test_client() -> request sent, waiting for auth");

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
        println!("test_client() -> auth granted, connection successful, listening for messages");
        let mut sub = rc.as_pubsub();
        sub.subscribe("clients:msg:all")?;
        sub.subscribe(["clients:msg:", &u].concat())?;

        loop {
            let msg = sub.get_message()?;
            let text: String = msg.get_payload()?;
            println!("test_client() -> msg seen: {}", text);
        }
    } else {
        println!("test_client() -> auth failed, timed out")
    }
    
    Ok(())
}