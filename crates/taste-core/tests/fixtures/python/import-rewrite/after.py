from myapp.users.models import User


def find(user_id):
    return User.get(user_id)
