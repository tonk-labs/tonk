//! Help text for Carry.

pub const MAIN_LONG_ABOUT: &str = "\
CLI for Dialog DB - a local-first semantic database.

Carry stores data as claims: (the: relation, of: entity, is: value). Claims are 
organized into domains (namespaced like 'com.myapp.person') and can be grouped 
into concepts (reusable schemas). All data lives in a .carry/ repository.

KEY CONCEPTS:
  Repo        A .carry/ repository containing your data
  Entity      Anything with an identity (has a DID like did:key:z...)
  Claim       A single fact: the X of Y is Z
  Domain      A namespace for relations (e.g., 'com.myapp.person')
  Concept     A reusable schema grouping attributes (e.g., 'person')
  Attribute   A typed relation (e.g., 'name' as Text, 'age' as Integer)

ASSERTED NOTATION:
  Carry uses YAML (or JSON) with a three-level structure:
    <entity>:
      <domain-or-concept>:
        <field>: <value>

  This format is used for both query output and file/stdin input.

EAV TRIPLE FORMAT:
  Use --format triples for a flat, pipeable YAML format:
    - the: <namespace/field>
      of: <entity-did>
      is: <value>

  This format is used for piping between carry commands:
    carry query person --format triples | carry assert -
    carry query person --format triples | carry retract -

NAMING:
  Use @name in assert commands to give an entity a human-readable name.
  Names assert dialog.meta/name on the entity and can be used as bookmarks.

META-SCHEMA:
  Domains starting with 'dialog.' are reserved for Dialog DB internals.
  Pre-registered concepts: attribute, concept, bookmark.";

pub const MAIN_AFTER_HELP: &str = "\
QUICK START:
  # Initialize a new repository
  carry init my-project

  # Define an attribute with a name
  carry assert attribute @person-name \\
    the=com.app.person/name as=Text cardinality=one

  # Define a concept referencing named attributes
  carry assert concept @person \\
    description='A person' with.name=person-name with.age=person-age

  # Or define via YAML file
  carry assert schema.yaml

  # Assert data using a domain
  carry assert com.app.person name=Alice age=28

  # Query data
  carry query com.app.person name age

  # Filter results
  carry query com.app.person name=\"Alice\" age

COMMON WORKFLOWS:
  # Update an existing entity
  carry assert com.app.person this=did:key:zAlice age=29

  # Retract a field
  carry retract com.app.person this=did:key:zAlice age

  # Assert from a file (asserted notation)
  carry assert claims.yaml

  # Pipe query output back as assert input (EAV triples)
  carry query person --format triples | carry assert -

  # Pipe query output to retract matching claims
  carry query person name=\"Alice\" --format triples | carry retract -

  # Pipe default YAML output (asserted notation also accepted)
  carry query com.app.person name age | carry assert -

For detailed help on any command: carry help <command>";

// -----------------------------------------------------------------------------
// Init
// -----------------------------------------------------------------------------

pub const INIT_LONG_ABOUT: &str = "\
Creates a new Dialog DB repository at .carry/ in the target directory.

If --repo is not specified, the repository is created in $PWD. If a name is
provided, it is asserted as the repository label.

If a repository already exists, reports its status.

The command generates an Ed25519 keypair for the repository, creating a unique
DID (e.g., did:key:z...). The private key is stored in .carry/<did>/credentials.

Pre-registered concepts (attribute, concept, bookmark) are bootstrapped during
init so they can be queried and used immediately.";

pub const INIT_AFTER_HELP: &str = "\
EXAMPLES:
  # Initialize in current directory
  carry init

  # Initialize with a label
  carry init my-project

  # Initialize in a specific directory
  carry init --repo /path/to/project

  # Initialize with label in specific directory
  carry init my-project --repo /path/to/project

OUTPUT:
  Initialized my-project repository in /path/to/.carry/did:key:z...";

// -----------------------------------------------------------------------------
// Query
// -----------------------------------------------------------------------------

pub const QUERY_LONG_ABOUT: &str = "\
Query entities by domain or concept, returning matching claims in asserted notation.

TARGET TYPES:
  Domain query (target contains '.')
    Searches for entities with claims in that domain. You choose which fields 
    to include in output.
    
  Concept query (target has no '.')
    Resolves the named concept via dialog.meta/name and returns all fields the
    concept defines. Specify fields only to filter, not to select output.

FIELD SYNTAX:
  name          Output field - include in results without filtering
  name=\"value\"   Filter field - only return entities matching this value

  Filter fields narrow results; output fields expand what's shown.";

pub const QUERY_AFTER_HELP: &str = "\
EXAMPLES:
  # Domain query: get name and age for all entities in com.app.person
  carry query com.app.person name age

  # Domain query with filter: only entities where name is Alice
  carry query com.app.person name=\"Alice\" age

  # Concept query: get all fields of 'person' concept for matching entities
  carry query person

  # Concept query with filter
  carry query person name=\"Alice\"

  # Output as JSON
  carry query person --format json

  # Output as EAV triples (for piping into assert/retract)
  carry query person --format triples

  # Pipe query results into assert
  carry query person --format triples | carry assert -

OUTPUT FORMATS:
  --format yaml (default):
    did:key:zAlice:
      com.app.person:
        name: Alice
        age: 28

  --format triples:
    - the: com.app.person/name
      of: did:key:zAlice
      is: Alice
    - the: com.app.person/age
      of: did:key:zAlice
      is: 28

  --format json:
    [{\"id\": \"did:key:zAlice\", \"name\": \"Alice\", \"age\": 28}]";

// -----------------------------------------------------------------------------
// Assert
// -----------------------------------------------------------------------------

pub const ASSERT_LONG_ABOUT: &str = "\
Assert claims on entities. Claims are facts stored as (the: relation, of: entity, is: value).

INPUT MODES:
  Target mode     carry assert <domain-or-concept> [@name] [this=<ENTITY>] field=value ...
  File mode       carry assert <file.yaml>
  Stdin mode      carry assert -

TARGET SYNTAX:
  Contains '.'    Domain - fields expand to domain/field relations
  No '.'          Concept - fields are validated against the concept schema

ENTITY NAMING:
  @name           Asserts dialog.meta/name on the entity. Use to give entities
                  human-readable names (e.g., @person-name, @person).

ENTITY SELECTION:
  Without this=   Creates a new entity (DID printed to stdout)
  With this=      Targets an existing entity

BUILTIN CONCEPTS:
  attribute       the=<relation> as=<Type> cardinality=<one|many>
  concept         with.<field>=<attr-name> [maybe.<field>=<attr-name>]
  bookmark        this=<DID> name=<bookmark-name>

FILE/STDIN FORMAT:
  Accepts two YAML formats, auto-detected:
  
  EAV triples (from --format triples):
    - the: <namespace/field>
      of: <entity-did>
      is: <value>
  
  Asserted notation (from default --format yaml):
    <entity-did>:
      <namespace>:
        <field>: <value>
  
  Also accepts JSON EAV triples:
    [{\"the\": \"...\", \"of\": \"...\", \"is\": ...}]";

pub const ASSERT_AFTER_HELP: &str = "\
EXAMPLES:
  # Assert using a domain (creates new entity)
  carry assert com.app.person name=Alice age=28
  # Output: did:key:zNewEntity

  # Define an attribute with a name
  carry assert attribute @person-name \\
    the=com.app.person/name as=Text cardinality=one

  # Define a concept referencing named attributes
  carry assert concept @person \\
    description='A person' with.name=person-name with.age=person-age

  # Update an existing entity
  carry assert com.app.person this=did:key:zAlice age=29

  # Assert from a YAML file (asserted notation)
  carry assert schema.yaml

  # Assert from stdin / pipe query output back (EAV triples)
  carry query person --format triples | carry assert -

  # Assert from stdin (asserted notation also works)
  carry query person name=\"Alice\" | carry assert -

ATTRIBUTE FIELDS:
  the             Relation identifier (e.g., com.app.person/name)
  as              Value type (Text, UnsignedInteger, Boolean, Entity, etc.)
  cardinality     one (default) or many
  description     Human-readable description (optional)

CONCEPT FIELDS:
  with.<field>    Required field referencing an attribute by name or selector
  maybe.<field>   Optional field referencing an attribute by name or selector
  description     Human-readable description (optional)

  If a with/maybe value contains '/', it is treated as an attribute selector
  (the attribute is looked up or auto-created). Without '/', it is treated
  as an attribute bookmark name.

FILE VS TARGET DETECTION:
  - '-' is always stdin
  - Contains '/' or ends in .yaml/.yml/.json -> file path
  - Otherwise -> target (domain or concept)";

// -----------------------------------------------------------------------------
// Retract
// -----------------------------------------------------------------------------

pub const RETRACT_LONG_ABOUT: &str = "\
Retract claims from entities. Removes facts from the database.

INPUT MODES:
  Target mode     carry retract <domain-or-concept> this=<entity> field[=value] ...
  File mode       carry retract <file.yaml>
  Stdin mode      carry retract -

FIELD SYNTAX:
  field           Retract the claim for this field regardless of value
  field=value     Retract only if the claim matches this exact value
                  (useful for cardinality:many attributes with multiple values)

Unlike assert, retract typically requires this= to identify which entity to modify.";

pub const RETRACT_AFTER_HELP: &str = "\
EXAMPLES:
  # Retract a field (any value)
  carry retract person this=did:key:zAlice age

  # Retract specific value (for multi-valued fields)
  carry retract person this=did:key:zAlice tag=urgent

  # Retract using domain
  carry retract com.app.person this=did:key:zAlice name age

  # Retract from file
  carry retract retractions.yaml

  # Retract from stdin
  cat to_remove.yaml | carry retract -

DIFFERENCE FROM ASSERT:
  Assert adds or updates claims; retract removes them.
  Other claims on the entity are unaffected.";

// -----------------------------------------------------------------------------
// Status
// -----------------------------------------------------------------------------

pub const STATUS_LONG_ABOUT: &str = "\
Display information about the current repository.

Shows the resolved .carry/ repository path and DID.";

pub const STATUS_AFTER_HELP: &str = "\
EXAMPLES:
  # Show status
  carry status

  # Show status as JSON
  carry status --format json

OUTPUT:
  Repo: /path/to/project/.carry
  DID: did:key:z...";

// -----------------------------------------------------------------------------
// Space
// -----------------------------------------------------------------------------

pub const SPACE_LONG_ABOUT: &str = "\
Manage spaces within a .carry/ repository.

Spaces are isolated namespaces within a single repo, each with its own
Ed25519 identity and data store. Use spaces to keep workstreams separate
within the same project.

Each space has a unique DID (e.g., did:key:z...) and an optional human-readable
label. The active space is the default target for query, assert, and retract
commands. Use --space <DID|LABEL> on any command to target a specific space
without switching.";

pub const SPACE_AFTER_HELP: &str = "\
EXAMPLES:
  # List all spaces
  carry space list

  # Create a new space with a label
  carry space create research

  # Switch to a space by label
  carry space switch research

  # Switch to a space by DID
  carry space switch did:key:zAbc123

  # Show current active space
  carry space active

  # Delete a space (with confirmation)
  carry space delete research

  # Delete without confirmation
  carry space delete research --yes

  # Query in a specific space without switching
  carry query person --space research

SPACE RESOLUTION:
  Commands accept either a DID or a label to identify a space.
  If a label matches multiple spaces, an error is returned —
  use the DID to be specific.";
