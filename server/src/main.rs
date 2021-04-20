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

    tokio::task::spawn(async{ if let Err(e) = heartbeat().await {panic!("heartbeat() -> {}", e)} });
    tokio::task::spawn(async{ if let Err(e) = client_handler().await {panic!("client_handler() -> {}", e)} });
    tokio::task::yield_now().await;

    loop {
        let heartbeat: i64 = redis::cmd("get").arg("heartbeat").query_async(&mut rc).await.unwrap_or(0);
        let timestamp = chrono::Utc::now().timestamp();

        let diff = timestamp - heartbeat;

        if diff > 60 {
            eprintln!("Heartbeat is behind by {} seconds. Assuming death and ending process.", diff);
            std::process::exit(1);
        }

        std::thread::sleep(std::time::Duration::from_secs(10));
    }
}

async fn heartbeat() -> Result<(), Box<dyn std::error::Error>> {
    let mut rc = REDIS_CLIENT.get_tokio_connection().await?;
    loop {
        redis::cmd("set").arg("heartbeat").arg(chrono::Utc::now().timestamp()).query_async(&mut rc).await?;
        let msg = Msg {
            stamp: chrono::Utc::now().timestamp(),
            from: "server".to_owned(),
            event: "heartbeat".to_owned(),
            .. Msg::default()
        };
        redis::cmd("publish").arg("clients:msg:all").arg(toml::to_string(&msg)?).query_async(&mut rc).await?;
        redis::cmd("publish").arg("handlers:msg:all").arg(toml::to_string(&msg)?).query_async(&mut rc).await?;

        redis::cmd("setnx").arg("reset").arg(true).query_async(&mut rc).await?;
        let reset: bool = redis::cmd("get").arg("reset").query_async(&mut rc).await?;
        if reset {
            eprintln!("{} -> Database reset invoked.", chrono::Utc::now().timestamp());
            redis::cmd("set").arg("reset").arg(false).query_async(&mut rc).await?;
            redis::cmd("flushall").query_async(&mut rc).await?;
            redis::cmd("acl").arg("load").query_async(&mut rc).await?;
            redis::cmd("set").arg("audit").arg(false).query_async(&mut rc).await?;
        }

        redis::cmd("setnx").arg("audit").arg(true).query_async(&mut rc).await?;
        let audit: bool = redis::cmd("get").arg("audit").query_async(&mut rc).await?;
        if audit {
            eprintln!("{} -> Client audit invoked.", chrono::Utc::now().timestamp());
            redis::cmd("set").arg("audit").arg(false).query_async(&mut rc).await?;
            // do audit of dead client records and acls
        }

        // heartbeat interval is purposefully arbitrary to avoid predictions
        // heartbeat exists only to monitor handlers and provide server status for clients
        let mut rng = rand::thread_rng();
        std::thread::sleep(std::time::Duration::from_secs(rand::Rng::gen_range(&mut rng, 10..20)));
    }
}

async fn client_handler() -> Result<(), Box<dyn std::error::Error>> {
    let mut sub_rc = REDIS_CLIENT.get_connection()?;
    let mut sub = sub_rc.as_pubsub();

    sub.subscribe("handlers:msg:all")?;
    sub.subscribe("clients:msg:connecting")?;

    loop {
        let msg_string: String = sub.get_message()?.get_payload()?;
        tokio::task::spawn(async{ if let Err(e) = msg_handler(msg_string).await {panic!("msg_handler() -> {}", e)} });
    }
}

async fn msg_handler(msg_string: String) -> Result<(), Box<dyn std::error::Error>> {
    let mut rc = REDIS_CLIENT.get_tokio_connection().await?;
    let msg: Msg = toml::from_str(&msg_string)?;
    if msg.action == "auth_request" {
        if msg.from.len() == 0 || msg.data.len() == 0 { return Ok(()) };
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
    if msg.event == "heartbeat" && msg.from == "server" {
        
    }
    Ok(())
}
