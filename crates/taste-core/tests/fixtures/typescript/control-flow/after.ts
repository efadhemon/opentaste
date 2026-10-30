function label(user: { name: string } | null) {
  if (!user) return "guest";
  return user.name;
}
