# Frozen behavior contracts

These files are **golden copies** of user-visible arrangement that the Platform
refactor must not change: toolbar order, keyboard bindings, shell mount order and
inspector section order.

Rules:

- A contract file is a **literal duplicate**, never an import of the production
  table. Importing the production value would make its test tautological.
- Changing a contract file is changing the product. It is allowed only with an
  explicit user-visible-change decision recorded in `TODO.md` — never to make a
  refactor pass.
- The *probe* (the test that reads the production side) may change when the
  mechanism changes — e.g. when the toolbar becomes registry-derived, the probe
  reads the registry instead of the static array. The contract itself must not.
