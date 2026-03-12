//! Help text for Carry.

pub const MAIN_LONG_ABOUT: &str = "\
CLI for Dialog DB - a local-first semantic database.

Carry stores data as claims: (the: relation, of: entity, is: value). Claims are 
organized into domains (namespaced like 'com.myapp.person') and can be grouped 
into concepts (reusable schemas). All data lives in a .carry/ repository.

KEY CONCEPTS:
  Space       A .carry/ repository containing your data
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

  This format is used for query output. File/stdin input currently requires
  formal triple format (see 'carry help assert').";

pub const MAIN_AFTER_HELP: &str = "\
QUICK START:
  # Initialize a new space
  carry init my-project

  # Define a schema (concept with attributes)
  carry assert concept description='A person' \\
    with.name.the=com.app.person/name with.name.as=Text with.name.cardinality=one \\
    with.age.the=com.app.person/age with.age.as=UnsignedInteger with.age.cardinality=one

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
  carry assert person this=did:key:zAlice age=29

  # Retract a field
  carry retract person this=did:key:zAlice age

  # Assert from a file (formal triple format)
  carry assert claims.yaml

For detailed help on any command: carry help <command>";

// -----------------------------------------------------------------------------
// Init
// -----------------------------------------------------------------------------

pub const INIT_LONG_ABOUT: &str = "\
Creates a new Dialog DB repository at .carry/ in the target directory.

If --site is not specified, the repository is created in $PWD. If a repository 
already exists at that location and a name is provided, the name is asserted 
as the space label.

The command generates an Ed25519 keypair for the space, creating a unique 
space DID (e.g., did:key:zSpace). The private key is stored in 
.carry/<space-did>/credentials.";

pub const INIT_AFTER_HELP: &str = "\
EXAMPLES:
  # Initialize in current directory
  carry init

  # Initialize with a label
  carry init my-project

  # Initialize in a specific directory
  carry init --site /path/to/project

  # Initialize with label in specific directory
  carry init my-project --site /path/to/project

OUTPUT:
  Initialized my-project repository in /path/to/.carry/did:key:zAbc123";

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
    Resolves the named concept via bookmark and returns all fields the concept 
    defines. Specify fields only to filter, not to select output.

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

  # Compose queries with + (join on same entity)
  carry query com.app.person name=\"Alice\" + com.app.user email

  # Output as JSON
  carry query person --format json

OUTPUT FORMAT (asserted notation):
  did:key:zAlice:
    com.app.person:
      name: Alice
      age: 28

  did:key:zBob:
    com.app.person:
      name: Bob
      age: 35";

// -----------------------------------------------------------------------------
// Assert
// -----------------------------------------------------------------------------

pub const ASSERT_LONG_ABOUT: &str = "\
Assert claims on entities. Claims are facts stored as (the: relation, of: entity, is: value).

INPUT MODES:
  Target mode     carry assert <domain-or-concept> field=value ...
  File mode       carry assert <file.yaml>
  Stdin mode      carry assert -

TARGET SYNTAX:
  Contains '.'    Domain - fields expand to domain/field relations
  No '.'          Concept - fields are validated against the concept schema

ENTITY SELECTION:
  Without this=   Creates a new entity (DID printed to stdout)
  With this=      Targets an existing entity

FILE/STDIN FORMAT:
  Accepts formal triple format: a YAML/JSON sequence of {the, of, is} objects.
  Format is auto-detected for stdin (JSON if starts with '{' or '[').";

pub const ASSERT_AFTER_HELP: &str = "\
EXAMPLES:
  # Assert using a domain (creates new entity)
  carry assert com.app.person name=Alice age=28
  # Output: did:key:zNewEntity

  # Assert using a concept (creates new entity)
  carry assert person name=Bob age=35

  # Update an existing entity
  carry assert person this=did:key:zAlice age=29

  # Assert from a YAML file
  carry assert schema.yaml

  # Assert from stdin (formal triple format)
  cat claims.yaml | carry assert -

YAML FILE FORMAT (formal triples):
  File/stdin input uses formal triple format, NOT asserted notation.
  Each triple is a {the, of, is} object:

  - the: com.app.person/name
    of: did:key:zAlice
    is: Alice

  - the: com.app.person/age
    of: did:key:zAlice
    is: 28

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
Display information about the current space and repository.

Shows the resolved .carry/ repository path, space DID, and label (if set).
Useful for verifying which space commands will operate on.";

pub const STATUS_AFTER_HELP: &str = "\
EXAMPLES:
  # Show status
  carry status

  # Show status as JSON
  carry status --format json

OUTPUT:
  Site: /path/to/project/.carry/did:key:zSpace
  Space: did:key:zSpace
  Label: my-project";
