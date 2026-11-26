; issue: tonk's issue library (issue.yaml) in the notation. A line
; starting `; GAP:` says what the notation has no spelling for, and the
; YAML lines that need it, as in notebook.clj.
;
; Every line is `cardinality: :one`: the YAML's fields give no pick, and
; tonk reads such a field's newest claim.
;
; GAP: an attribute declared on its own. issue.yaml declares six
; `attribute!:` (YAML 1-52); the notation declares an attribute only as
; a line of a concept or a command. Five are the lines of `issue` below.
; `issue/blocks` (YAML 47-52, `pick: :all`) is a line of no concept, so
; it has no spelling here and is left out.
;
; GAP: a field's own description. The YAML gives each field one (YAML
; 58-101); the notation keeps the comment above a line as the line's
; description, so each is written there.

; Work items with priorities, types, labels, and dependencies
(concept (issue ?i title: ?title description: ?description type: ?type
                priority: ?priority status: ?status)
  ; Issue title
  (io.gozala.issue/title of: ?i is: ?title as: :text cardinality: :one)
  ; Description of the issue
  (io.gozala.issue/description of: ?i is: ?description as: :text cardinality: :one)
  ; Type of issue represented by an entity acting like a category.
  ; While it is open ended and you can have any kind of isues we recognize following ones
  ;
  ; bug:. - Something broken than needs fixing
  ; feature:. - New functionality
  ; task:. - Work item (tests, docs, refactoring)
  ; epic:. -  Large feature with subtasks
  ; chore:. - Maintenance (dependencies, tooling)
  (io.gozala.issue/type of: ?i is: ?type as: :entity cardinality: :one)
  ; Priority for the issue as an entity we recognize following
  ;
  ; critical:. - Security, data loss, broken builds
  ; high:. - Major features, important bugs
  ; medium:. - Nice-to-have features, minor bugs
  ; low:. - Polish, optimization
  ; backlog:. - Future ideas
  (io.gozala.issue/priority of: ?i is: ?priority as: :entity cardinality: :one)
  ; Status of this issue we recognize following
  ;
  ; open:.
  ; fixed:.
  ; progress:.
  ; canceled:.
  (io.gozala.issue/status of: ?i is: ?status as: :entity cardinality: :one))
