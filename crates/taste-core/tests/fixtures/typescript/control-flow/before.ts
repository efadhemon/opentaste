function label(user: { name: string } | null) {
  if (user) {
    return user.name;
  } else {
    return "guest";
  }
}
