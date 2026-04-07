pub mod category;
pub mod image;

use actix_web::web;

pub fn configure(cfg: &mut web::ServiceConfig) {
    image::configure(cfg);
    category::configure(cfg);
}
