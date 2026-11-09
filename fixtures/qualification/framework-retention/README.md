# Framework retention diagnostic operands

These files are captured operands from an experimental native Framework enclosure
history. They are imported as diagnostic BRep geometry in the regression; they do
not contain native feature history and do not establish equivalence to the source.

The original public source is Framework Computer Inc.'s
[ExpansionCard_ThreadedInsert.stp](https://github.com/FrameworkComputer/ExpansionCards/blob/2904a95d60d4ba771c5c3c56d6fdba7078243fe8/Mechanical/Printable/3D/ExpansionCard_ThreadedInsert.stp),
licensed [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). Original STEP SHA-256:
`6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09`.
The captured operands were produced by OneCAD worker SHA-256
`4272493274b789539a0e731afe0d45084e344ccf646f46bec35af6561a3fcfb0`.

| File | Meaning | SHA-256 |
|---|---|---|
| `p.brep` | Retention operand P, derived from B intersected with a slab | `f215fd6dc72943da29fbfbdfdc3136bc7d95306d264b7284c11967d888bd34bf` |
| `sharp.brep` | Sharp-region operand S | `65602a458049e58d7e8193169d1abf45b97c63ac32783537c7cbddd61a5c6355` |
| `c69.brep` | Cylindrical cutting operand C69 | `137820d1a285b08c9e8a9ddaccc895be56acf494b0f76ed7189ebe5875279c61` |

The operands retain their captured coordinates and BinTools v4 bytes. The test
covers the local set rewrite only; it does not qualify the complete enclosure.
