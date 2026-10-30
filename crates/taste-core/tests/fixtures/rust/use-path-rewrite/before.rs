use super::models::User;

pub fn find(id: u64) -> Option<User> {
    User::get(id)
}
