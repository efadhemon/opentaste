import * as yup from "yup";
import { z } from "zod";

export const UserSchema = yup.object({
  name: yup.string().required(),
});

export const OrderSchema = z.object({ id: z.string() });
