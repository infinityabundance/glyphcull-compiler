# GlyphCull Terminology Standard

**Canonical.** Every repository, all code, all docs, all commit messages, all review
comments use this terminology. The codebase reads like a graphics engine — never like a
browser. Terms from the web stack (DOM, page, text cache, visible paragraphs, render page)
are forbidden.

## 1. Mandatory vocabulary

| Term | Meaning | Never say |
|---|---|---|
| **chunk** | The atomic unit of the compiled document (Chunk Graph node) | node, element, block |
| **caption** | A renderable chunk carrying short label text (figure captions; the HTML table `<caption>`, which both runtimes lay out above the rows) | subtitle, legend |
| **document** | The whole compiled document instance held by a runtime | page, article |
| **package** | The `.cull` artifact on disk / in transit | file, bundle, archive |
| **load** | Bring a package into a runtime and validate it | parse, fetch |
| **stream chunk** | Make a chunk's content available to the runtime (from the package) | load text, fetch text |
| **materialize chunk** | Transform a chunk into renderable glyphs/layout | decode paragraph, render paragraph |
| **materialization** | The subsystem that materializes chunks | text engine, layout engine (as a whole) |
| **visibility system** | Determines what should exist right now (viewport + semantic culling) | viewport manager, cull pass (only), page logic |
| **viewport culling** | Geometric visibility determination against the viewport | scrolling logic, visible paragraphs |
| **semantic culling** | Semantic visibility determination (hidden chunks, mode) | filtering, hiding |
| **visible set** | The set of chunks currently determined visible | visible paragraphs, on-screen content |
| **materialization queue** | Ordered, budgeted work list of chunks to materialize | task queue, to-do list |
| **glyph cache** | Cache of generated glyph instances | text cache, font cache |
| **glyph generation** | Producing positioned glyph instances from atlas + layout | text rendering (as generation), shaping only |
| **draw list** | Ordered GPU commands produced from the visible set | render tree, paint list |
| **build draw list** | Produce the draw list | render page, paint frame |
| **paint** | Execute the draw list on the GPU | render, draw (in API docs) |
| **select** | Establish a selection range | highlight text (as the API) |
| **copy** | Extract selection as plain text | clipboard (as the API) |
| **chunk lifecycle** | The chunk state machine (Compressed → Queued → Materializing → Visible → Cooling → Evicted) | caching states, GC states |
| **evict** | Return a chunk's resources after the cooling period | free, delete, garbage collect |
| **atlas** | MSDF glyph atlas (or image atlas) | texture page, sprite sheet (in docs) |
| **SDF / MSDF** | (Multi-channel) signed distance field | distance map |
| **MSDF sign convention** | The canonical direction of an MSDF channel (SPEC.md §2.5, normative): **< 0.5 outside glyph, == 0.5 glyph edge, > 0.5 inside glyph**. Positive distance points into the glyph. Every generator, shader, CPU reference compositor, and fixture agrees; no subsystem inverts at render time. | — |
| **reference rasterizer** | The deterministic CPU rasterizer used for validation | golden renderer, software renderer |
| **glyph instance** | One positioned glyph occurrence in layout | character box, glyph quad |

## 2. Prohibited vocabulary

Browser/web-stack terms are prohibited in code identifiers, comments, docs, and messages:

- DOM, DOM tree, element, node (use chunk), HTML tree (use semantic graph / chunk graph)
- page, paginate (a *viewport* exists; documents are continuous streams)
- text cache (glyph cache), text renderer (glyph generation / paint)
- visible paragraphs (visible set)
- render page (build draw list / paint)
- decode paragraph (materialize chunk)
- load text (stream chunk)
- cascade, CSS (the compiler owns CSS translation; runtimes never mention CSS)
- layout engine, text engine (say materialization, line breaking, block layout)

## 3. Verb discipline

- The runtime **loads** a package, **streams** chunks, **materializes** chunks, **builds**
  a draw list, **paints** the draw list, **selects**, **copies**, **destroys**.
- Culling **determines**; it never generates, never lays out, never paints.
- Materialization **transforms** chunks into glyphs; it never culls.
- The compiler **translates**, **resolves**, **emits**; it never renders.

## 4. Enforcement

- Reviewers reject contributions violating this standard (see CONTRIBUTING.md).
- Docs are audited against this standard in the Phase 5 documentation audit.
