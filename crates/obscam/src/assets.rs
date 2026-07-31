use axum::{
    body::Body,
    http::{Response, header},
};

const INDEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../web/dist/index.html"
));
const APP: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../web/dist/app.js"
));
const CONTROL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../web/dist/control.js"
));
const MODEL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../web/dist/model.js"
));
const PRESENTATION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../web/dist/presentation.js"
));
const WHEP: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../web/dist/whep.js"
));
const STYLES: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../web/dist/styles.css"
));

pub(crate) async fn index() -> Response<Body> {
    response("text/html; charset=utf-8", INDEX)
}

pub(crate) async fn app() -> Response<Body> {
    response("text/javascript; charset=utf-8", APP)
}

pub(crate) async fn control() -> Response<Body> {
    response("text/javascript; charset=utf-8", CONTROL)
}

pub(crate) async fn model() -> Response<Body> {
    response("text/javascript; charset=utf-8", MODEL)
}

pub(crate) async fn presentation() -> Response<Body> {
    response("text/javascript; charset=utf-8", PRESENTATION)
}

pub(crate) async fn whep() -> Response<Body> {
    response("text/javascript; charset=utf-8", WHEP)
}

pub(crate) async fn styles() -> Response<Body> {
    response("text/css; charset=utf-8", STYLES)
}

fn response(content_type: &'static str, content: &'static str) -> Response<Body> {
    Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(content))
        .expect("static asset response headers are valid")
}
