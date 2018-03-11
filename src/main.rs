#![feature(plugin)]
#![plugin(rocket_codegen)]

mod db;
mod models;
mod schema;
mod stats;
mod worker;

extern crate chrono;
#[macro_use]
extern crate diesel;
extern crate dotenv;
#[macro_use]
extern crate lazy_static;
extern crate r2d2_diesel;
extern crate regex;
extern crate rocket;
#[macro_use]
extern crate rocket_contrib;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;

use std::env;

use dotenv::dotenv;
use regex::RegexSet;
use rocket_contrib::{Json, Value};
use rocket::Request;
use rocket::State;
use rocket::http::Status;
use rocket::response::{Responder, Response};
use self::diesel::prelude::*;

use db::DbConn;
use schema::entries as entries_db;
use worker::Worker;

pub const ORGANIZATION_ROOT: &str = "https://github.com/apertium";
pub const ORGANIZATION_RAW_ROOT: &str = "https://raw.githubusercontent.com/apertium";
pub const LANG_CODE_RE: &str = r"\w{2,3}(_\w+)?";

fn normalize_name(name: &str) -> String {
    if name.starts_with("apertium-") {
        name.to_owned()
    } else {
        format!("apertium-{}", name)
    }
}

fn match_name(name: &str) -> bool {
    lazy_static! {
        static ref RE: RegexSet = RegexSet::new(&[
            format!(r"^apertium-({re})$", re=LANG_CODE_RE),
            format!(r"^apertium-({re})-({re})$", re=LANG_CODE_RE),
        ]).unwrap();
    }

    RE.matches(name).matched_any()
}

enum JsonResult {
    Err(Option<Json<Value>>, Status),
    Ok(Json<Value>),
}

impl<'r> Responder<'r> for JsonResult {
    fn respond_to(self, req: &Request) -> Result<Response<'r>, Status> {
        match self {
            JsonResult::Ok(value) => value.respond_to(req),
            JsonResult::Err(maybe_value, status) => match maybe_value {
                Some(value) => match value.respond_to(req) {
                    Ok(mut response) => {
                        response.set_status(status);
                        Ok(response)
                    }
                    err => err,
                },
                None => Err(status),
            },
        }
    }
}

// TODO: allow as of request

#[get("/")]
fn index() -> &'static str {
    "
    USAGE
      GET /apertium-<code1>(-<code2>)
        retrieves statistics for the specified package
      GET /apertium-<code1>(-<code2>)/<kind>
        retrieves <kind> statistics for the specified package
      POST /apertium-<code1>(-<code2>)
        calculates statistics for the specified package
      POST /apertium-<code1>(-<code2>)/<kind>
        calculates <kind> statistics for the specified package
    "
}

#[get("/<name>")]
fn get_stats(name: String, conn: DbConn, worker: State<Worker>) -> JsonResult {
    let normalized_name = normalize_name(&name);
    if match_name(&normalized_name) {
        let maybe_entries = entries_db::table
            .filter(entries_db::name.eq(&name))
            .order(entries_db::created)
            .limit(1)
            .load::<models::Entry>(&*conn);
        if let Ok(entries) = maybe_entries {
            if entries.is_empty() {
                if let Some(in_progress_tasks) = worker.get_tasks_in_progress(&name) {
                    JsonResult::Err(
                        Some(Json(json!({
                        "name": normalized_name,
                        "in_progress": in_progress_tasks,
                    }))),
                        Status::TooManyRequests,
                    )
                } else {
                    match worker.launch_tasks(&name, None) {
                        Ok((ref new_tasks, ref _in_progress_tasks)) if new_tasks.is_empty() => {
                            JsonResult::Err(
                                Some(Json(json!({
                                "name": normalized_name,
                                "error": "No statistics could be found",
                            }))),
                                Status::NotFound,
                            )
                        }
                        Ok((ref _new_tasks, ref in_progress_tasks)) => JsonResult::Err(
                            Some(Json(json!({
                                "name": normalized_name,
                                "in_progress": in_progress_tasks,
                            }))),
                            Status::Accepted,
                        ),
                        Err(error) => JsonResult::Err(
                            Some(Json(json!({
                                "name": normalized_name,
                                "error": error,
                            }))),
                            Status::BadRequest,
                        ),
                    }
                }
            } else {
                let maybe_entries = entries_db::table
                    .filter(entries_db::name.eq(name))
                    .group_by(entries_db::kind)
                    .order(entries_db::created)
                    .load::<models::Entry>(&*conn);
                if let Ok(entries) = maybe_entries {
                    JsonResult::Ok(Json(json!({
                        "name": normalized_name,
                        "stats": entries,
                    })))
                } else {
                    JsonResult::Err(None, Status::InternalServerError)
                }
            }
        } else {
            JsonResult::Err(None, Status::InternalServerError)
        }
    } else {
        JsonResult::Err(
            Some(Json(json!({
            "name": normalized_name,
            "error": format!("Invalid package format: {}", name),
        }))),
            Status::BadRequest,
        )
    }
}

#[get("/<name>/<kind>")]
fn get_specific_stats(name: String, kind: String) -> String {
    // TODO: validate kind
    format!("{}: {}", name, kind)
    // TODO: implement this
}

#[post("/<name>")]
fn calculate_stats(name: String) -> String {
    name
    // TODO: implement this
}

#[post("/<name>/<kind>")]
fn calculate_specific_stats(name: String, kind: String) -> String {
    format!("{}: {}", name, kind)
    // TODO: implement this
}

fn main() {
    dotenv().ok();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = db::init_pool(&database_url);
    let worker = Worker::new(pool.clone());

    rocket::ignite()
        .manage(pool)
        .manage(worker)
        .mount(
            "/",
            routes![
                index,
                get_stats,
                get_specific_stats,
                calculate_stats,
                calculate_specific_stats,
            ],
        )
        .launch();
}
