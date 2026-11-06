mod rule;
mod spirit;
mod status;

use crate::app::{Data, Error};

pub fn all() -> Vec<poise::Command<Data, Error>> {
    vec![rule::rule(), spirit::spirit(), status::status()]
}
