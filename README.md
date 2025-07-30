# HTTP_Server

This is a simple webapp written in Rust without any framework showing you all the main "things" you want to be able to do written this kink of thing.

So we got some:

- Accepting clients, basic TCP, and multithreading
- Account creation, custom concurrent database, learning about `Arc<Mutex<File>>`
- Update infos, fast updates, with a `SeekFrom` and same length lines in the database
- SignIn, SignOut, cookies in http headers and extracting it
- Session cookie, fast verification and deletion with `HashSet<[u8; 32]>`
- Image, favicon transfer, understanding how browsers will download the images, with good http header inormations

Note:

This webapp architecture is based on a SaaS, with a potential paywall to request some work to do internally or to another server with custom APIs. A user has a predefine amount of tokens that can be redefined at line `771` (the last field).

# Run it

```
$ git clone https://github.com/julienlargetpiet/HTTP_Server
$ cd HTTP_Server/src
$ cargo run
```

Now go to your browser and type: `127.0.0.1:8080`

# Architectural details

## TCP connection

Simple enough, each new request has its own TCP connection (POST, GET...), the TCP connection is terminated when `handle_request()` ends.

`handle_request()` is charged to determine the logic to operate based on the content to the HTTP request, method, path, Cookies...

`handle_request()` is launched inside a ThreadPool to allow concurrent client requests to operate concurently, up to 15 (this parameter can be changed at line `949`)

In fact the ThreadPool is the manager and has 15 workers that accpets any task. The task is always to proceed `handle_request()`. But of course when a job is launched, it can be sent to just one worker and a worker that is not busy, it is why this job is mutex protected, so just workers that has nothing to do can receive it, and only the first one to receive it that can execute the job.

The TCP connection is automtically closed at the end of `handle_request()` because it owns it, so when it terminates, the current TCP connection goes out of scope and closes, disappears.

# `handle_request()`

Basically acts like a `marshalling yard` to call appropriate functions that corresponds to the intended action described by the client (browser) HTTP request

## Database `src/databases/db.txt`

The database is composed of:

- username (12 max chars)
- password (15 max chars)
- email (20 max users)
- tokens used (10 max chars -> length of `MAX::u64`)
- maximum tokens (10 max chars -> length of `MAX::u64`)

## Database `src/databases/sessions.txt`

- session token of exactly 64 characters long

## `db.txt` and concurrent acccess

main function => **creating mutexes** for 3 different actions read, write append (which is a simplified read action for adding elements)

for each accepted TCP connection => the shared pointer containing the mutexes for the 3 different action to possibly operate in the database is cloned and passed to the handle_request function

when `handle_request()` detects that it needs to operate on the database, it call the appropriate function `add_user(), incr_tokens(), verify_credentials()` pass the `Arc<Mutex<File>>` that correponds to the appropriate actions to do.

Because the databases is being read by other workers, we need to be sure that when we start reading from it, the cursor is at the beginning, so we use `SeekFrom(0)` before reading it.

## `db.txt` and fast updates

When we want to change the amount of token a user has used, we have to search for the right line.

Pretty simple right, we just compare the beginning of each line to the intended username (since the first field is username).

Then we we found it, instead of changing its line and buffering al contents in an array and then rewrite all in the file, we just change this line. But wait to be able to do this, since a file can be modeled as a buffer, we have to make sure that when we are jumping from a line to another, we stepped from a constant amount of bytes. It is why we have to make sure that all the lines are same length.

So when a user is added, we have to padd its username, password and email to correspond to its maximum field size. We pad it with blank spaces. This happens in `add_user()`.

## Session tokens and fast verification

What can we do to have an amotised `O(1)` way of being able to verify that something is in a set????

Right !!! **HashSet** of something

But wait HashSet are just alive in the programm memory, what to do if we Ctrl-C the server, so all the HashSet data is gone ??? 

Yess technically, but we can just intercept the Ending Signal that is sent by the Ctrl-C and execute a function here. You nailed it, in this function we will store the HashSet data in a permanant file that is `src/databases/sessions.txt` and when the server will start, one of te first thing we wil do is to load the sessions in this database in the HashSet.

Ok, but of what type is the HashSet ???


