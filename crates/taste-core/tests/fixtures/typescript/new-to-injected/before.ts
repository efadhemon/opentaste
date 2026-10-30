import { Injectable } from "@nestjs/common";
import { UsersRepository } from "./users.repository";

@Injectable()
export class UsersService {
  private readonly repo = new UsersRepository();

  find(id: string) {
    return this.repo.find(id);
  }
}
