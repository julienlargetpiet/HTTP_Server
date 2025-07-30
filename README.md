# HTTP_Server

This is a simple webapp written in Rust without any framework showing you all the main "things" you want to be able to do written this kink of thing.

So we got some:

- Accepting clients, basic TCP, and multithreading
- Account creation, custom concurrent database, learning about `Arc<Mutex<File>>`
- Update infos, fast updates, with a `SeekFrom` and same length lines in the database
- SignIn, SignOut, cookies in http headers and extracting it
- Session cookie, fast verification and deletion with `HashSet<[u8; 32]>`
- Image, favicon transfer, understanding how browsers will download the images, with good http header inormations


