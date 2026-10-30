import * as yup from "yup";

export const UserSchema = yup.object({
  name: yup.string().required(),
});
