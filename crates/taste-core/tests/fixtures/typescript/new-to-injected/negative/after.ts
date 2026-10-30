import { Injectable } from "./container";
import { UsersRepository } from "./users.repository";

@Injectable()
export class UsersService {
  constructor(private readonly repo: UsersRepository) {}

  find(id: string) {
    return this.repo.find(id);
  }
}
