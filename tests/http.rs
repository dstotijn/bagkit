mod common;

use bagkit::{api, query};
use common::{ImportOptions, assert_matches_schema, import};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

struct RunningServer(Child);
impl Drop for RunningServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Reply {
    status: u16,
    /// Header lines, lower-cased.
    headers: String,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        let prefix = format!("{}: ", name.to_ascii_lowercase());
        self.headers
            .lines()
            .find_map(|line| line.strip_prefix(&prefix))
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

fn send(port: u16, method: &str, target: &str, headers: &str) -> Reply {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        stream,
        "{method} {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{headers}\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let (status_line, headers) = head.split_once("\r\n").unwrap_or((head, ""));
    Reply {
        status: status_line
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap(),
        headers: headers.to_ascii_lowercase(),
        body: body.to_owned(),
    }
}

fn request(port: u16, method: &str, target: &str) -> Reply {
    send(port, method, target, "")
}

fn assert_closed(stream: &mut TcpStream) {
    let mut bytes = [0; 1];
    match stream.read(&mut bytes) {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ) => {}
        result => panic!("expected connection to close, got {result:?}"),
    }
}

fn convert(db: &Path, address_list: Option<bagkit::dataset::Format>) {
    import(
        &[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bag.xml")],
        &ImportOptions {
            output: db.to_owned(),
            date: "2026-10-07".into(),
            force: false,
            cache_mib: 8,
            geometry: true,
            threads: 0,
            address_list,
        },
    )
    .unwrap();
}

fn start_server(db: &Path, options: &[&str]) -> (RunningServer, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let server = RunningServer(
        Command::new(env!("CARGO_BIN_EXE_bagkit"))
            .args(["serve", &port.to_string()])
            .args(["--host", "127.0.0.1", "--workers", "2", "--database"])
            .arg(db)
            .args(options)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut ready = false;
    for _ in 0..200 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ready, "HTTP service did not start");
    (server, port)
}

const RD_NEW: &str = "http://www.opengis.net/def/crs/EPSG/0/28992";

#[test]
fn live_http_serves_the_documented_api() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("bag.sqlite");
    convert(&db, None);
    let (server, port) = start_server(&db, &[]);
    let database = query::open(&db).unwrap();
    for (target, schema) in [
        ("/v1", "Root"),
        ("/v1/", "Root"),
        ("/v1/crss", "Crss"),
        ("/v1/adressen?postcode=7311KZ", "AdresCollectie"),
        (
            "/v1/adressen?postcode=7311kz&huisnummer=110",
            "AdresCollectie",
        ),
        (
            "/v1/adressen?postcode=7311KZ&huisnummer=110&huisletter=a",
            "AdresCollectie",
        ),
        (
            "/v1/adressen?postcode=7311KZ&huisnummer=110&huisnummertoevoeging=2&exacteMatch=true",
            "AdresCollectie",
        ),
        ("/v1/adressen?postcode=9999ZZ", "AdresCollectie"),
        (
            &format!("/v1/adressen?postcode=7311KZ&crs={RD_NEW}"),
            "AdresCollectie",
        ),
        ("/v1/adressen/0000200000000002", "Adres"),
        // Withdrawn addresses are found by identifier.
        ("/v1/adressen/0000200000000004", "Adres"),
        (
            "/v1/adressen/_dichtstbij?punt=5.962442171959101,52.211733682888124",
            "AdresCollectie",
        ),
        (
            &format!(
                "/v1/adressen/_dichtstbij?punt=194315.783,469449.074&puntCrs={RD_NEW}&crs={RD_NEW}"
            ),
            "AdresCollectie",
        ),
        ("/v1/adressen/_dichtstbij?punt=0,0", "AdresCollectie"),
    ] {
        let reply = request(port, "GET", target);
        assert_eq!(reply.status, 200, "{target}: {}", reply.body);
        assert_eq!(reply.header("content-type"), Some("application/json"));
        assert_eq!(reply.header("api-version"), Some(api::VERSION));
        assert_eq!(reply.header("access-control-allow-origin"), Some("*"));
        let expected = api::get(&database, target).unwrap();
        assert_eq!(reply.body, expected.body, "{target}");
        assert_matches_schema(schema, &reply.json());
        if schema.starts_with("Adres") {
            let crs = if target.contains("crs=") {
                RD_NEW
            } else {
                "http://www.opengis.net/def/crs/ogc/1.3/crs84"
            };
            assert_eq!(
                reply.header("content-crs"),
                Some(format!("<{}>", crs.to_ascii_lowercase()).as_str()),
                "{target}"
            );
        }
    }
    let address = request(port, "GET", "/v1/adressen/0000200000000004").json();
    assert_eq!(address["status"], "Naamgeving ingetrokken");
    let rd = request(
        port,
        "GET",
        &format!("/v1/adressen/0000200000000001?crs={RD_NEW}"),
    )
    .json();
    assert_eq!(
        rd["adresseerbaarObject"]["geometrie"]["coordinates"],
        serde_json::json!([194315.783, 469449.074])
    );

    let description = request(port, "GET", "/v1/openapi.json");
    assert_eq!(description.status, 200);
    assert_eq!(description.body, api::OPENAPI);
    assert_eq!(description.header("access-control-allow-origin"), Some("*"));

    for (method, target, status, headers) in [
        ("GET", "/", 404, ""),
        ("GET", "/7311KZ/110", 404, ""),
        ("GET", "/v1/adressen/", 404, ""),
        ("GET", "/v1/adressen/0000200000000099", 404, ""),
        ("GET", "/v1/adressen", 400, ""),
        (
            "GET",
            "/v1/adressen?postcode=7311KZ&houseNumber=110",
            400,
            "",
        ),
        ("GET", "/v1/adressen/_dichtstbij?punt=5.9", 400, ""),
        ("GET", "/v1/adressen/123", 400, ""),
        ("POST", "/v1/adressen?postcode=7311KZ", 405, ""),
        ("DELETE", "/v1", 405, ""),
        ("GET", "/v1", 406, "Accept: text/html\r\n"),
    ] {
        let reply = send(port, method, target, headers);
        assert_eq!(reply.status, status, "{method} {target}");
        assert_eq!(
            reply.header("content-type"),
            Some("application/problem+json"),
            "{method} {target}"
        );
        assert_eq!(reply.header("api-version"), Some(api::VERSION));
        assert_eq!(reply.json()["status"], status);
        assert_matches_schema("Problem", &reply.json());
    }
    assert_eq!(
        request(port, "POST", "/v1").header("allow"),
        Some("get, head")
    );
    let invalid = request(port, "GET", "/v1/adressen?postcode=0000AA&huisnummer=x").json();
    assert_eq!(invalid["invalidParams"].as_array().unwrap().len(), 2);

    let head = request(port, "HEAD", "/v1/adressen?postcode=7311KZ");
    assert_eq!(head.status, 200);
    assert_eq!(head.body, "");
    assert_eq!(head.header("api-version"), Some(api::VERSION));

    let handles: Vec<_> = (0..16)
        .map(|_| {
            thread::spawn(move || {
                request(port, "GET", "/v1/adressen?postcode=7311KZ&huisnummer=110")
            })
        })
        .collect();
    for handle in handles {
        let reply = handle.join().unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.json()["adressen"].as_array().unwrap().len(), 3);
    }
    drop(server);
}

#[test]
fn connection_limit_and_deadline_release_idle_and_partial_header_clients() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("bag.sqlite");
    convert(&db, None);
    let (_server, port) = start_server(
        &db,
        &["--max-connections", "4", "--connection-timeout-secs", "1"],
    );
    // Allow the readiness probe to disconnect before filling the connection slots.
    thread::sleep(Duration::from_millis(30));
    let mut clients = Vec::new();
    for _ in 0..3 {
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        client
            .write_all(b"GET /v1/adressen?postcode=7311KZ HTTP/1.1\r\nHost: ")
            .unwrap();
        clients.push(client);
    }
    let target = "/v1/adressen?postcode=7311KZ&huisnummer=110";
    assert_eq!(request(port, "GET", target).status, 200);
    // Keep-alive responses are allowed, but their idle connections also expire.
    let mut keepalive = TcpStream::connect(("127.0.0.1", port)).unwrap();
    keepalive
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    keepalive
        .write_all(b"GET /v1 HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut bytes = [0; 1024];
    assert!(keepalive.read(&mut bytes).unwrap() > 0);
    thread::sleep(Duration::from_millis(30));
    let mut excess = TcpStream::connect(("127.0.0.1", port)).unwrap();
    excess
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let _ = excess.write_all(b"GET /v1 HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert_closed(&mut excess);
    for mut client in clients {
        assert_closed(&mut client);
    }
    let mut remaining = Vec::new();
    keepalive.read_to_end(&mut remaining).unwrap();
    assert_eq!(request(port, "GET", target).status, 200);
}

#[test]
fn slow_reader_does_not_hold_database_workers_or_connection_slots_indefinitely() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("bag.sqlite");
    convert(&db, None);
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(
            "WITH RECURSIVE n(i) AS (
                 VALUES (1)
                 UNION ALL
                 SELECT i + 1
                 FROM n
                 WHERE i < 10000
             )
             INSERT INTO nummeraanduidingen
             SELECT
                 900000000000 + i,
                 '7777AA',
                 huisnummer,
                 huisletter,
                 huisnummertoevoeging,
                 status,
                 openbare_ruimte_identificatie,
                 woonplaats_identificatie,
                 adresseerbaar_object_identificatie,
                 indicatie_nevenadres
             FROM nummeraanduidingen, n
             WHERE identificatie = 200000000001;",
        )
        .unwrap();
    let (_server, port) = start_server(
        &db,
        &["--max-connections", "2", "--connection-timeout-secs", "2"],
    );
    thread::sleep(Duration::from_millis(30));
    let mut slow = TcpStream::connect(("127.0.0.1", port)).unwrap();
    slow.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    slow.write_all(b"GET /v1/adressen?postcode=7777AA HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let target = "/v1/adressen?postcode=7311KZ&huisnummer=110";
    assert_eq!(request(port, "GET", target).status, 200);
    thread::sleep(Duration::from_secs(3));
    let mut bytes = [0; 65536];
    loop {
        match slow.read(&mut bytes) {
            Ok(0) => break,
            Ok(_) => continue,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                ) =>
            {
                break;
            }
            Err(error) => panic!("slow response did not close: {error}"),
        }
    }
    assert_eq!(request(port, "GET", target).status, 200);
}

#[test]
fn address_lists_answer_address_routes_and_refuse_nearest_lookups() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("adreslijst.sqlite");
    convert(&db, Some(bagkit::dataset::Format::Sqlite));
    let (_server, port) = start_server(&db, &[]);
    let reply = request(
        port,
        "GET",
        "/v1/adressen?postcode=7311KZ&huisnummer=110&huisletter=A",
    );
    assert_eq!(reply.status, 200);
    assert_matches_schema("AdresCollectie", &reply.json());
    assert_eq!(reply.json()["adressen"].as_array().unwrap().len(), 2);
    let reply = request(port, "GET", "/v1/adressen/0000200000000001");
    assert_eq!(reply.status, 200);
    assert_matches_schema("Adres", &reply.json());
    let root = request(port, "GET", "/v1").json();
    assert_eq!(root["dataset"]["indeling"], "adreslijst");
    let reply = request(
        port,
        "GET",
        "/v1/adressen/_dichtstbij?punt=5.9624422,52.2117337",
    );
    assert_eq!(reply.status, 501);
    assert_matches_schema("Problem", &reply.json());
    assert_eq!(
        reply.json()["detail"],
        "zoeken op afstand vereist een database met tabellen; deze database is een adreslijst"
    );
}
