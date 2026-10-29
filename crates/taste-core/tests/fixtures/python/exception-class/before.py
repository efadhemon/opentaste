from .errors import UserNotFound


def find(users, user_id):
    if user_id not in users:
        raise Exception(f"no user {user_id}")
    return users[user_id]
