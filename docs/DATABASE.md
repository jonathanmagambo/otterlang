# Database Drivers for OtterLang

OtterLang provides database access through Rust FFI, leveraging existing Rust database drivers. This approach allows us to use mature, well-tested database libraries without reimplementing them.

## Supported Databases

### SQLite (via `rusqlite`)

SQLite support is provided through the `rusqlite` crate.

**Usage:**

```otter
use rust:rusqlite
use json

fn main:
    # Open database
    result = rusqlite.open("test.db")
    data = json.parse(result)
    
    if data["ok"]:
        handle = data["handle"]
        
        # Create table
        rusqlite.execute(handle, "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        
        # Insert data
        rusqlite.execute(handle, "INSERT INTO users (name) VALUES ('Alice')")
        rusqlite.execute(handle, "INSERT INTO users (name) VALUES ('Bob')")
        
        # Query data
        query_result = rusqlite.query(handle, "SELECT * FROM users")
        rows = json.parse(query_result)
        if rows["ok"]:
            for row in rows["rows"]:
                println(f"User ID: {row[\"id\"]}, Name: {row[\"name\"]}")
        
        # Close connection
        rusqlite.close(handle)
    else:
        println(f"Error: {data[\"error\"]}")
```

**Functions:**

- `rusqlite.open(path: string) -> string`: Opens a SQLite database. Returns JSON with `{"ok": true, "handle": "..."}` or `{"ok": false, "error": "..."}`
- `rusqlite.execute(handle: string, sql: string) -> string`: Executes a SQL statement. Returns JSON with `{"ok": true, "rows": n}` or error
- `rusqlite.query(handle: string, sql: string) -> string`: Executes a query. Returns JSON with `{"ok": true, "rows": [...]}` or error
- `rusqlite.close(handle: string) -> unit`: Closes a database connection

### PostgreSQL (via `postgres`)

PostgreSQL support is provided through the `postgres` crate.

**Usage:**

```otter
use rust:postgres
use json

fn main:
    # Connect to database
    conn_str = "postgresql://user:password@localhost/mydb"
    result = postgres.connect(conn_str)
    data = json.parse(result)
    
    if data["ok"]:
        handle = data["handle"]
        
        # Execute query
        query_result = postgres.query(handle, "SELECT * FROM users WHERE age > $1")
        rows = json.parse(query_result)
        if rows["ok"]:
            for row in rows["rows"]:
                println(f"User: {row[\"name\"]}")
        
        # Close connection
        postgres.close(handle)
    else:
        println(f"Error: {data[\"error\"]}")
```

**Functions:**

- `postgres.connect(conn_string: string) -> string`: Connects to PostgreSQL. Connection string format: `postgresql://user:password@host/database`. Returns JSON with handle or error
- `postgres.execute(handle: string, sql: string) -> string`: Executes a SQL statement. Returns JSON with rows affected or error
- `postgres.query(handle: string, sql: string) -> string`: Executes a query. Returns JSON with results array or error
- `postgres.close(handle: string) -> unit`: Closes a database connection

## Implementation Details

Both database drivers are integrated via the OtterLang FFI system:

1. **Bridge Configuration**: Each database crate has a `bridge.yaml` file in `ffi/<crate-name>/` that describes how to call Rust functions
2. **Automatic Compilation**: When you `use rust:rusqlite` or `use rust:postgres`, OtterLang automatically:
   - Downloads the Rust crate via Cargo
   - Generates FFI bindings
   - Compiles the bridge library
   - Makes functions available to your OtterLang code

3. **Connection Management**: Connections are managed internally using thread-local storage, with handles returned as strings

## Benefits of This Approach

- **No Reimplementation**: We leverage existing, battle-tested Rust database drivers
- **Automatic Updates**: When Rust crates update, you get the benefits automatically
- **Performance**: Direct FFI access means minimal overhead
- **Flexibility**: Easy to add more database drivers by creating new bridge configurations

## Adding More Database Drivers

To add support for another database:

1. Create `ffi/<crate-name>/bridge.yaml` with function definitions
2. The FFI system will automatically handle compilation and integration
3. Use in your code with `use rust:<crate-name>`

See existing bridge.yaml files in `ffi/rusqlite/` and `ffi/postgres/` for examples.
