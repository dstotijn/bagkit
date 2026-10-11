use crate::{api, query};
use anyhow::{Context, Result, ensure};
use http_body_util::Full;
use hyper::{Request, Response, body::Bytes, server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use std::{
    convert::Infallible,
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tokio::sync::{Semaphore, mpsc, oneshot};

const QUERY_FAILED: &str = "database query failed";
const UNAVAILABLE: &str = "the database workers are not available";

struct Job {
    route: api::Route,
    reply: oneshot::Sender<api::Response>,
}

pub fn serve(
    database: &Path,
    bind: &str,
    workers: usize,
    max_connections: usize,
    connection_timeout: Duration,
) -> Result<()> {
    ensure!(workers > 0, "workers must be positive");
    ensure!(
        max_connections > 0 && max_connections <= Semaphore::MAX_PERMITS,
        "max-connections is out of range"
    );
    ensure!(
        !connection_timeout.is_zero(),
        "connection timeout must be positive"
    );
    let capacity = workers.checked_mul(2).context("too many workers")?;
    let connections = (0..workers)
        .map(|_| query::open(database))
        .collect::<Result<Vec<_>>>()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(bind).await?;
        eprintln!(
            "Serving {} on http://{} with {workers} workers, at most {max_connections} connections",
            database.display(),
            listener.local_addr()?
        );
        let (sender, receiver) = mpsc::channel::<Job>(capacity);
        let receiver = Arc::new(Mutex::new(receiver));
        for (index, db) in connections.into_iter().enumerate() {
            let receiver = receiver.clone();
            thread::Builder::new()
                .name(format!("bagkit-query-{index}"))
                .spawn(move || {
                    loop {
                        let job = receiver.lock().unwrap().blocking_recv();
                        let Some(job) = job else { break };
                        if job.reply.is_closed() {
                            continue;
                        }
                        let reply = api::respond(&db, &job.route).unwrap_or_else(|error| {
                            eprintln!("Query failed: {error:#}");
                            api::problem(500, QUERY_FAILED)
                        });
                        let _ = job.reply.send(reply);
                    }
                })?;
        }
        let permits = Arc::new(Semaphore::new(max_connections));
        loop {
            let (socket, _) = listener.accept().await?;
            let Ok(permit) = permits.clone().try_acquire_owned() else {
                // Close excess sockets immediately rather than queueing connections.
                continue;
            };
            socket.set_nodelay(true)?;
            let sender = sender.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let service = service_fn(move |request| respond(request, sender.clone()));
                let mut http = http1::Builder::new();
                http.max_headers(64).max_buf_size(16384);
                // A total lifetime also bounds slow headers, idle keep-alive and writes.
                let _ = tokio::time::timeout(
                    connection_timeout,
                    http.serve_connection(TokioIo::new(socket), service),
                )
                .await;
            });
        }
    })
}

async fn respond(
    request: Request<hyper::body::Incoming>,
    sender: mpsc::Sender<Job>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let target = request.uri().path_and_query().map_or("/", |p| p.as_str());
    let accept = request
        .headers()
        .get(hyper::header::ACCEPT)
        .map(|value| value.to_str().unwrap_or(""));
    let reply = match api::route(request.method().as_str(), target, accept) {
        Err(reply) => reply,
        Ok(route) => match api::respond_static(&route) {
            Some(reply) => reply,
            None => {
                let (reply, result) = oneshot::channel();
                if sender.send(Job { route, reply }).await.is_ok() {
                    result
                        .await
                        .unwrap_or_else(|_| api::problem(503, UNAVAILABLE))
                } else {
                    api::problem(503, UNAVAILABLE)
                }
            }
        },
    };
    let mut response = Response::builder().status(reply.status);
    for (name, value) in reply.headers {
        response = response.header(name, value);
    }
    Ok(response.body(Full::new(Bytes::from(reply.body))).unwrap())
}
