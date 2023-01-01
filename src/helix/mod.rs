mod user;
pub use user::*;

mod stream;
pub use stream::*;

mod auth;
pub use auth::*;

pub struct Helix {
    //user_buf: Vec<UserCredentials>
}

impl Helix {
    pub fn get_user_from_id(&self, id: impl ToString) -> &User {
        unimplemented!()
    }

    pub fn get_user_from_login(&self, login: impl ToString) -> &User {
        unimplemented!()
    }
}