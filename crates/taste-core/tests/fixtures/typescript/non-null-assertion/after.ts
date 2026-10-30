export function greet(user?: User): string {
  return `Hello ${user?.name ?? "guest"}`;
}
