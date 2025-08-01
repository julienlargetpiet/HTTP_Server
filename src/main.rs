use std::net::{TcpListener, TcpStream, Shutdown};
use std::io::{Read, Write, BufReader, BufRead, SeekFrom, Seek};
use std::thread;
use std::sync::{mpsc, Arc, Mutex};
use std::collections::HashSet;
use std::fs::{OpenOptions, File};
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};

enum Message {
  NewJob(Job),
  Terminate,
}

struct Worker {
  id: usize,
  thread: Option<thread::JoinHandle<()>>,
}

type Job = Box<dyn FnOnce() + Send + 'static>;

impl Worker {
  fn new(id: usize, receiver: Arc<Mutex<mpsc::Receiver<Message>>>) -> Worker {
    let thread = thread::spawn(move || loop {
      let msg_type = receiver
          .lock()
          .unwrap()
          .recv()
          .unwrap();
      
      match msg_type {
        Message::NewJob(job) => job(),
        Message::Terminate => {break;},
      }

    });

    Worker {
      id,
      thread: Some(thread),
    }
  }
}

struct ThreadPool {
  workers: Vec<Worker>,
  sender: mpsc::Sender<Message>,
}

impl ThreadPool {

  pub fn new(size: usize) -> ThreadPool {
    assert!(size > 0);

    let (sender, receiver) = mpsc::channel();
    let receiver = Arc::new(Mutex::new(receiver));

    let mut workers: Vec<Worker> = Vec::with_capacity(size);

    for id in 0..size {
      workers.push(Worker::new(id, Arc::clone(&receiver)));
    }

    ThreadPool { workers, sender }
  }

  pub fn execute<F>(&self, f: F)
  where
    F: FnOnce() + Send + 'static,
  {
    let job = Box::new(f);
    self.sender.send(Message::NewJob(job)).unwrap();
  }
}

impl Drop for ThreadPool {
  fn drop(&mut self) {
    println!("Sending termination signals to all workers.");

    for _ in &self.workers {
      self.sender.send(Message::Terminate).unwrap();
    }

    for worker in &mut self.workers {
      println!("Shutting down worker {}", worker.id);
      if let Some(thread) = worker.thread.take() {
        thread.join().unwrap();
      }
    }
  }
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(64);

    for &byte in bytes {
        hex.push(HEX_CHARS[(byte >> 4) as usize] as char); 
        hex.push(HEX_CHARS[(byte & 0x0F) as usize] as char);
    }

    hex
}

fn url_decode(input: &String) -> String {
  let mut output = String::new();
  let mut chars = input.chars().peekable();

  while let Some(c) = chars.next() {
    if c == '%' {
      let hex1 = chars.next();
      let hex2 = chars.next();

      if let (Some(h1), Some(h2)) = (hex1, hex2) {
        let hex_str = format!("{}{}", h1, h2);
        if let Ok(byte) = u8::from_str_radix(&hex_str, 16) {
          output.push(byte as char);
          continue;
        }
      }
      output.push('%');
      if let Some(h1) = hex1 { output.push(h1); }
      if let Some(h2) = hex2 { output.push(h2); }
    } else if c == '+' {
      output.push(' ');
    } else {
      output.push(c);
    }
  }
  output
}

fn handle_request(mut stream: TcpStream, 
                  db_fappend_mutex: Arc<Mutex<File>>,
                  db_fread_mutex: Arc<Mutex<File>>,
                  session_tokens_mutex: Arc<Mutex<HashSet<[u8; 32]>>>,
                  db_fwrite_mutex: Arc<Mutex<File>>) {

  let mut buffer: [u8; 1024] = [0; 1024];
  stream.read(&mut buffer).unwrap();
  println!("Request {}", String::from_utf8_lossy(&buffer[..]));

  let expected_index: &[u8; 16] = b"GET / HTTP/1.1\r\n";
  let expected_to_fetch: &[u8; 24] = b"GET /to_fetch HTTP/1.1\r\n";
  let expected_favicon: &[u8; 27] = b"GET /favicon.ico HTTP/1.1\r\n";
  let expected_create_account: &[u8; 30] = b"GET /create_account HTTP/1.1\r\n";
  let expected_faq: &[u8; 19] = b"GET /faq HTTP/1.1\r\n";
  let expected_documentation: &[u8; 29] = b"GET /documentation HTTP/1.1\r\n";
  let expected_reviews: &[u8; 23] = b"GET /reviews HTTP/1.1\r\n";
  let expected_created_account: &[u8; 32] = b"POST /created_account HTTP/1.1\r\n";
  let expected_signin: &[u8; 22] = b"GET /signin HTTP/1.1\r\n";
  let expected_signout: &[u8; 23] = b"GET /signout HTTP/1.1\r\n";
  let expected_connected: &[u8; 26] = b"POST /connected HTTP/1.1\r\n";
  let expected_req: &[u8; 27] = b"POST /do_the_req HTTP/1.1\r\n";
  let expected_img: &[u8; 36] = b"GET /media/img/ev_ui4.jpg HTTP/1.1\r\n";

  let mut content: String = String::new();
  let mut binary_content: Vec<u8> = Vec::<u8>::new();
  
  let (action_id, is_binary) = 
      if buffer.starts_with(expected_index) {
        (0, false)
      } else if buffer.starts_with(expected_to_fetch) {
        (1, false)
      } else if buffer.starts_with(expected_favicon) {
        (0, true)
      } else if buffer.starts_with(expected_create_account) {
        (2, false)
      } else if buffer.starts_with(expected_faq) {
        (3, false)
      } else if buffer.starts_with(expected_documentation) {
        (4, false)
      } else if buffer.starts_with(expected_reviews) {
        (5, false)
      } else if buffer.starts_with(expected_created_account) {
        (6, false)
      } else if buffer.starts_with(expected_signin) {
        (7, false)
      } else if buffer.starts_with(expected_signout) {
        (8, false)
      } else if buffer.starts_with(expected_connected) {
        (9, false)
      } else if buffer.starts_with(expected_req) {
        (10, false)
      } else if buffer.starts_with(expected_img) {
        (1, true)
      } else {
        (-1, false)
      };

  let resp: String;

  if !is_binary {
    match action_id {
      0 => {
             let mut req_string: String = String::from_utf8(
                                              buffer.to_vec()
                                              ).unwrap();
             req_string = req_string.trim_end_matches('\0').to_string();
             match retrieve_session(&req_string) {
               Ok((session_token, _)) => {
                 let mut cur_val: [u8; 32] = [0u8; 32];
                 for i in 0..32 {
                     cur_val[i] = u8::from_str_radix(&session_token[i * 2..i * 2 + 2], 16).unwrap();
                 }
                 let all_sessions = session_tokens_mutex.lock().unwrap();
                 println!("all_sessions: {:?}", all_sessions);
                 if all_sessions.contains(&cur_val) {
                   let mut file = OpenOptions::new()
                              .read(true)
                              .open("templates/connected_index.html")
                              .unwrap();
                   file.read_to_string(&mut content).unwrap();
                 } else {
                   let mut file = OpenOptions::new()
                              .read(true)
                              .open("templates/index.html")
                              .unwrap();
                   file.read_to_string(&mut content).unwrap();
                 }
               },
               Err(_) => {
                 let mut file = OpenOptions::new()
                              .read(true)
                              .open("templates/index.html")
                              .unwrap();
                 file.read_to_string(&mut content).unwrap();
               }
             }
            },
      1 => {
             thread::sleep(Duration::from_secs(5));
             content = r#"{"name": "Yolo", "age": 34}"#.to_string();
          },
      2 => {
             let mut file = OpenOptions::new()
                 .read(true)
                 .open("templates/create_account.html")
                 .unwrap();
             file.read_to_string(&mut content).unwrap();
           },
      3 => {
             let mut file = OpenOptions::new()
                 .read(true)
                 .open("templates/faq.html")
                 .unwrap();
             file.read_to_string(&mut content).unwrap();
           },
      4 => {
             let mut file = OpenOptions::new()
                 .read(true)
                 .open("templates/documentation.html")
                 .unwrap();
             file.read_to_string(&mut content).unwrap();
           },
      5 => {
             let mut file = OpenOptions::new()
                 .read(true)
                 .open("templates/reviews.html")
                 .unwrap();
             file.read_to_string(&mut content).unwrap();
           },
      6 => {
             let mut cred_string: String = String::from_utf8(
                                               buffer.to_vec())
                                               .unwrap();
             cred_string = cred_string.trim_end_matches('\0').to_string();
             let (username, password, email);
             match retrieve_sent_credentials(&cred_string) {
               Ok((usr, pass, eml)) => {
                 (username, password, email) = (usr, pass, eml);
               },
               Err(e) => {
                 resp = format!("HTTP/1.1 200 OK\r\n Content-Length: {}\r\n\r\n{}", 
                   e.len(), 
                   e);
                 stream.write(resp.as_bytes()).unwrap();
                 stream.flush().unwrap();
                 return;
               }
             }
             if let Err(e) = add_user(&username, 
                                      &password, 
                                      &email,
                                      &db_fappend_mutex,
                                      &db_fread_mutex) {
               resp = format!("HTTP/1.1 200 OK\r\n Content-Length: {}\r\n\r\n{}", 
                 e.len(), 
                 e);
               stream.write(resp.as_bytes()).unwrap();
               stream.flush().unwrap();
               return;
             }

             let mut file = OpenOptions::new()
                 .read(true)
                 .open("/dev/urandom")
                 .unwrap();
             let mut data: [u8; 32] = [0u8; 32];
             if let Err(e) = file.read_exact(&mut data)
                 .map_err(|_| "Unable to read from '/dev/urandom'".to_string()) {
               resp = format!("HTTP/1.1 200 OK\r\n Content-Length: {}\r\n\r\n{}", 
                   e.len(), 
                   e);
                 stream.write(resp.as_bytes()).unwrap();
                 stream.flush().unwrap();
             } else {
               let random_string: String = to_hex(&data);
               resp = format!("HTTP/1.1 302 Found\r\n\
                               Location: /\r\n\
                               Content-Length: 0\r\n\
                               Set-Cookie: session={}; HttpOnly; Path=/; Max-Age=316224000\r\n\
                               Set-Cookie: username={}; HttpOnly; Path=/; Max-Age=316224000\r\n\r\n",
                               random_string.clone(),
                               username.clone());
               stream.write(resp.as_bytes()).unwrap();
               stream.flush().unwrap();
               let mut all_sessions = session_tokens_mutex.lock().unwrap();
               all_sessions.insert(data);
             }
             return;
           },
      7 => {
             let mut file = OpenOptions::new()
                 .read(true)
                 .open("templates/signin.html")
                 .unwrap();
             file.read_to_string(&mut content).unwrap();
           },
      8 => {
             let req_string: String = String::from_utf8(
                                              buffer.to_vec()
                                              ).unwrap();
             match retrieve_session(&req_string) {
               Ok((session_token, _)) => {
                 let mut cur_val: [u8; 32] = [0u8; 32];
                 for i in 0..32 {
                     cur_val[i] = u8::from_str_radix(&session_token[i * 2..i * 2 + 2], 16).unwrap();
                 }
                 let mut all_sessions = session_tokens_mutex.lock().unwrap();
                 if all_sessions.contains(&cur_val) {
                   all_sessions.remove(&cur_val);
                 } else {
                   resp = format!("HTTP/1.1 200 OK\r\n Content-Length: 16\r\n\r\n Cookie not found");
                   stream.write(resp.as_bytes()).unwrap();
                   stream.flush().unwrap();
                   return;
                 }
               },
               Err(e) => {
                 resp = format!("HTTP/1.1 200 OK\r\n Content-Length: {}\r\n\r\n{}", 
                   e.len(), 
                   e);
                 stream.write(resp.as_bytes()).unwrap();
                 stream.flush().unwrap();
                 return;
               }
             }
             resp = format!("HTTP/1.1 302 Found\r\n\
                             Location: /\r\n\
                             Content-Length: 0\r\n\
                             \r\n");
             stream.write(resp.as_bytes()).unwrap();
             stream.flush().unwrap();
             return;
           },
      9 => {
             let mut cred_string: String = String::from_utf8(
                                               buffer.to_vec())
                                               .unwrap();

             cred_string = cred_string.trim_end_matches('\0').to_string();

             let session_token: String;

             let (mut username, mut password);
             match retrieve_sent_credentials_signin(&cred_string) {
               Ok((usr, pass)) => {
                 (username, password) = (usr, pass);
               },
               Err(e) => {
                 resp = format!("HTTP/1.1 200 OK\r\n Content-Length: {}\r\n\r\n{}", 
                    e.len(),
                    e);
                 stream.write(resp.as_bytes()).unwrap();
                 stream.flush().unwrap();
                 return;
               }
             }

             let mut repeat_n: usize = 12 - username.len();
             username = format!("{}{}", username, " ".repeat(repeat_n));
             repeat_n = 15 - password.len();
             password = format!("{}{}", password, " ".repeat(repeat_n));

             match retrieve_session(&cred_string) {
               Ok((session_tkn, _)) => {
                 session_token = session_tkn;
               },
               Err(e) => {
                 resp = format!("HTTP/1.1 200 OK\r\n Content-Length: {}\r\n\r\n{}", 
                    e.len(), 
                    e);
                 stream.write(resp.as_bytes()).unwrap();
                 stream.flush().unwrap();
                 return;
               }
             }

             let mut cur_val: [u8; 32] = [0u8; 32];
             for i in 0..32 {
                 cur_val[i] = u8::from_str_radix(&session_token[i * 2..i * 2 + 2], 16).unwrap();
             }

             if let Ok(_) = verify_credentials(&username, &password, &db_fread_mutex) {

               let mut file = OpenOptions::new()
                 .read(true)
                 .open("/dev/urandom")
                 .unwrap();

               let mut data: [u8; 32] = [0u8; 32];
               if let Err(e) = file.read_exact(&mut data)
                   .map_err(|_| "Unable to read from '/dev/urandom'".to_string()) {
                 resp = format!("HTTP/1.1 200 OK\r\n Content-Length: {}\r\n\r\n{}", 
                    e.len(), 
                    e);
                 stream.write(resp.as_bytes()).unwrap();
                 stream.flush().unwrap();
                 return;
               } else {
                 let random_string: String = to_hex(&data);
                 resp = format!("HTTP/1.1 302 Found\r\n\
                       Location: /\r\n\
                       Content-Length: 0\r\n\
                       Set-Cookie: session={}; HttpOnly; Path=/; Max-Age=316224000\r\n\
                       Set-Cookie: username={}; HttpOnly; Path=/; Max-Age=316224000\r\n\
                       \r\n",
                       random_string.clone(),
                       username.clone());
                 stream.write(resp.as_bytes()).unwrap();
                 stream.flush().unwrap();
                 let mut all_sessions = session_tokens_mutex.lock().unwrap();
                 if all_sessions.contains(&cur_val) {
                   all_sessions.remove(&cur_val);
                 }
                 all_sessions.insert(data);
               }
               return;
             }
             let mut file = OpenOptions::new()
                 .read(true)
                 .open("templates/not_valid_signin.html")
                 .unwrap();
             file.read_to_string(&mut content).unwrap();
           },
      10 => {
             let req_string: String = String::from_utf8(
                                              buffer.to_vec()
                                              ).unwrap();
             let username: String;
             match retrieve_session(&req_string) {
               Ok((session_token, usr)) => {
                 username = usr;
                 let mut cur_val: [u8; 32] = [0u8; 32];
                 for i in 0..32 {
                     cur_val[i] = u8::from_str_radix(&session_token[i * 2..i * 2 + 2], 16).unwrap();
                 }
                 let all_sessions = session_tokens_mutex.lock().unwrap();
                 if !all_sessions.contains(&cur_val) {
                   resp = format!("HTTP/1.1 200 OK\r\n Content-Length: 16\r\n\r\n Cookie not found");
                   stream.write(resp.as_bytes()).unwrap();
                   stream.flush().unwrap();
                   return;
                 }
               },
               Err(e) => {
                 resp = format!("HTTP/1.1 200 OK\r\n Content-Length: {}\r\n\r\n{}", 
                    e.len(), 
                    e);
                 stream.write(resp.as_bytes()).unwrap();
                 stream.flush().unwrap();
                 return;
               }
             }

             println!("ICIIIII");

             match do_the_req(&username, &db_fwrite_mutex, &db_fread_mutex) {
               Ok(_) => {
        
                println!("Success");

                 // do the actual req stuff, lie sending json data
                 content = r#"{"success": 1, "error": ""}"#.to_string();
               },
               Err(e) => {

                 println!("Error: {}", e);

                 if e == "Not enough tokens".to_string() {
                   content = r#"{"success": 0, "error": "not_enough"}"#.to_string();
                 } else {
                   content = format!(r#"{{"success": 0, "error": "{}"}}"#, e);
                 }
               },
             }
      },
      _ => { 
             stream.write_all(b"HTTP/1.1 404 NOT FOUND\r\n\
                           Content-Length: 17\r\n\
                           Content-Type: text/plain\r\n\
                           Connection: close\r\n\r\n404 Not Found Lol").unwrap();
             stream.flush().unwrap();
             stream.shutdown(Shutdown::Both).unwrap();
             println!("there");
             return;
           },
    };
    
    resp = format!("HTTP/1.1 200 OK\r\n Content-Length: {}\r\n\r\n{}", 
      content.len(), 
      content);
    stream.write(resp.as_bytes()).unwrap();
    stream.flush().unwrap();
  } else {
    let mut file: File;
    match action_id {
      0 => {
        file = OpenOptions::new()
                     .read(true)
                     .open("media/ico/ico.ico")
                     .unwrap();
      },
      1 => {
        file = OpenOptions::new()
                           .read(true)
                           .open("media/img/ev_ui4.jpg")
                           .unwrap();
      },
      _ => { 
             stream.write("HTTP/1.1 404 NOT FOUND".as_bytes()).unwrap();
             stream.flush().unwrap();
             return;
           },
    }
    file.read_to_end(&mut binary_content).unwrap(); 
    resp = format!("HTTP/1.1 200 OK\r\n\
                    Content-Type: image/x-icon\r\n\
                    Content-Length: {}\r\n\
                    Connection: close\r\n\r\n",
                    binary_content.len());
    stream.write(resp.as_bytes()).unwrap();
    stream.write(&binary_content).unwrap();
    stream.flush().unwrap();
  }
  return;
}

fn retrieve_sent_credentials_signin(x: &String) -> Result<(String, String), String> {
  println!("{}", x);
  let mut vec: Vec<String> = (*x).split("\r\n")
                         .map(|e| e.to_string())
                         .collect();
  let n: usize = vec.len();
  println!("{:?}", vec);
  if n == 0 {
    return Err("Bad request format".to_string());
  }
  vec[n - 1] = url_decode(&vec[n - 1]);
  vec = vec[n - 1].split("&")
                  .map(|e| e.to_string())
                  .collect();
  if vec.len() != 2 {
    return Err("Bad request format".to_string());
  }
  let vec1: Vec<String> = vec[0].split("=")
                                .map(|e| e.to_string())
                                .collect();
  if vec1.len() != 2 {
    return Err("Bad request format".to_string());
  }
  let vec2: Vec<String> = vec[1].split("=")
                                .map(|e| e.to_string())
                                .collect();
  if vec2.len() != 2 {
    return Err("Bad request format".to_string());
  }
  Ok((vec1[1].clone(), vec2[1].clone())) 
}


fn retrieve_sent_credentials(x: &String) -> Result<(String, String, String), String> {
  println!("{}", x);
  let mut vec: Vec<String> = (*x).split("\r\n")
                         .map(|e| e.to_string())
                         .collect();
  let n: usize = vec.len();
  println!("{:?}", vec);
  if n == 0 {
    return Err("Bad request format".to_string());
  }
  vec[n - 1] = url_decode(&vec[n - 1]);
  vec = vec[n - 1].split("&")
                  .map(|e| e.to_string())
                  .collect();
  if vec.len() != 3 {
    return Err("Bad request format".to_string());
  }
  let vec1: Vec<String> = vec[0].split("=")
                                .map(|e| e.to_string())
                                .collect();
  if vec1.len() != 2 {
    return Err("Bad request format".to_string());
  }
  let vec2: Vec<String> = vec[1].split("=")
                                .map(|e| e.to_string())
                                .collect();
  if vec2.len() != 2 {
    return Err("Bad request format".to_string());
  }
  let vec3: Vec<String> = vec[2].split("=")
                                .map(|e| e.to_string())
                                .collect();
  if vec3.len() != 2 {
    return Err("Bad request format".to_string());
  }
  Ok((vec1[1].clone(), vec2[1].clone(), vec3[1].clone())) 
}

fn validate_email_no_regex(email: &String) -> Result<(), String> {
  if email.len() > 20 {
    return Err("Email is too long, 20 chars max".to_string());
  }
  let allowed: HashSet<char> = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-@.".chars().collect();
  let explicitely_forbidden: HashSet<char> = "@.".chars().collect();
  
  if !email.chars().all(|e| allowed.contains(&e)) {
    return Err("The email contains unauthorized characters".to_string());
  }
  
  if explicitely_forbidden.contains(&email.chars().next().unwrap()) {
    return Err("The is not well formatted".to_string());
  }
  
  if explicitely_forbidden.contains(&email.chars().last().unwrap()) {
    return Err("The is not well formatted".to_string());
  }
  
  if let Some((i, _)) = email.char_indices().find(|&(_, e)| e == '@') {
    
    if explicitely_forbidden.contains(&email.chars().nth(i + 1).unwrap()) {
      return Err("The is not well formatted".to_string());
    }
    
    if i + 2 > email.len() {
      return Err("The is not well formatted".to_string());
    }
    
    let mut has_dot: bool = false;
    let mut prec_dot: bool = false;
    
    for chr in email.chars().skip(i + 2) {
      if chr == '@' {
        return Err("Just one '@' is allowed in email".to_string());
      } else if chr == '.' {
        if prec_dot {
          return Err("2 following dots can't be present in an email".to_string());
        }
        has_dot = true;
        prec_dot = true;
      } else {
        prec_dot = false;
      }
    }
    
    if !has_dot {
      return Err("No dots was found for the email host".to_string());
    }
    Ok(())
  } else {
    return Err("'@' was not found in the email provided".to_string());
  }
}

fn legitimate_credentials(username: & String,
                          password: &String,
                          email: &String) -> Result<(), String> {
  validate_email_no_regex(email)?;
   
  if (*username).len() == 0 {
    return Err("Username must be equal to at least 1 char"
              .to_string());
  }
  if (*username).len() > 12 {
    return Err("Username max len is 12 chars"
              .to_string());
  }
  if (*password).len() < 8 {
    return Err("Password must be equal to at least 8 chars"
               .to_string());
  }
  if (*password).len() > 15 {
    return Err("Password max len is 15"
               .to_string());
  }
 
  let ref_letters: [char; 52] = ['a', 'b', 'c', 'd', 'e',
                                 'f', 'g', 'h', 'i', 'j',
                                 'k', 'l', 'm', 'n', 'o',
                                 'p', 'q', 'r', 's', 't',
                                 'u', 'v', 'w', 'x', 
                                 'y', 'z',
                                 'A', 'B', 'C', 'D', 'E',
                                 'F', 'G', 'H', 'I', 'J',
                                 'K', 'L', 'M', 'N', 'O',
                                 'P', 'Q', 'R', 'S', 'T',
                                 'U', 'V', 'W', 'X',
                                 'Y', 'Z']; 
  let ref_symbols: [char; 22] = ['!', '^', '?', ':', '|',
                          '(', ')', '{', '}', '[',
                          ']', '\'', '-', '_', '&',
                          '$', '*', ',', '%', '+',
                          '<', '>'];
  let ref_numbers: [char; 10] = ['0', '1', '2', '3', '4', 
                             '5', '6', '7', '8', '9'];
  let mut cnt: u8 = 0;

  if (*username)
      .chars()
      .any(|e| e.is_whitespace() || !e.is_ascii() || e == ',') {
    return Err("Username should not contain any whitespace and must be ASCII only and should not contain ','".to_string());
  }
 
  if (*password)
      .chars()
      .any(|e| e.is_whitespace() || !e.is_ascii() || e == ',') {
    return Err("Password should not contain any whitespace and must be ASCII only".to_string());
  } 

  let password_vec: Vec<char> = (*password).chars().collect();

  for chr in &password_vec {
    for chr2 in &ref_letters {
      if *chr2 == *chr {
        if cnt == 2 {
          return Ok(());
        }
        cnt += 1;
        continue
      }
    }
    for chr2 in &ref_symbols {
      if *chr2 == *chr {
        if cnt == 2 {
          return Ok(());
        }
        cnt += 1;
        continue
      }
    }
    for chr2 in &ref_numbers {
      if *chr2 == *chr {
        if cnt == 2 {
          return Ok(());
        }
        cnt += 1;
        continue
      }
    }
  }
  Err("Password must countain letters, numbers and special characters"
      .to_string())
}

fn add_user(username: &String,
            password: &String,
            email: &String,
            db_fappend_mutex: &Arc<Mutex<File>>,
            db_fread_mutex: &Arc<Mutex<File>>) -> Result<(), String> {
  
  legitimate_credentials(username, password, email)?;
 
  let mut file_read = (*db_fread_mutex).lock().unwrap();
  file_read.seek(SeekFrom::Start(0)).unwrap();
  let reader: BufReader<&File> = BufReader::new(&*file_read);
  for line in reader.lines() {
    let cur_str: String = line.map_err(|_| "failed to read from 'db.txt'".to_string())?;
    if cur_str.starts_with(username) {
      return Err("Username already taken".to_string());
    }
  }

  let mut file = (*db_fappend_mutex).lock().unwrap();

  let mut n_rep: usize = 12 - (*username).len();
  let mut repeat_string: String = " ".repeat(n_rep);
  let username2: String = format!("{}{}", *username, repeat_string);

  n_rep = 15 - (*password).len();
  repeat_string = " ".repeat(n_rep);
  let password2: String = format!("{}{}", *password, repeat_string);

  n_rep = 20 - (*email).len();
  repeat_string = " ".repeat(n_rep);
  let email2: String = format!("{}{}", *email, repeat_string);

  let newline: String = format!("{},{},{},0000000000,0000000001",
                                 username2,
                                 password2,
                                 email2);
  writeln!(file, "{}", newline)
      .map_err(|_| "Error appending to db.txt".to_string())?;
  Ok(())
}

fn retrieve_session(req: &String) -> Result<(String, String), String> {
  let x: Vec<String> = (*req).split("\r\n")
                           .map(|e| e.to_string())
                           .collect();
  if x.len() > 40 {
    return Err("Request too long".to_string());
  }
  let bgn: String = "Cookie: ".to_string();
  for line in &x {
    if line.starts_with(&bgn) {
      let cookie_line: Vec<String> = line.split("=")
                                         .map(|e| e.to_string())
                                         .collect();
      if cookie_line.len() != 3 {
        return Err("Error in cookie format".to_string());
      } else if cookie_line[2].len() == 0 {
        return Err("Cookie format is not valid".to_string());
      }
      let session_token_vec: Vec<String> = cookie_line[1].split(";")
                                                          .map(|e| e.to_string())
                                                          .collect();
      if session_token_vec[0].len() != 64 {
        return Err("Cookie format is not valid".to_string());
      }
      return Ok((session_token_vec[0].clone(), cookie_line[2].clone()));
    }
  }
  Err("Cookie field was not found".to_string())
}

fn verify_credentials(username: &String, 
                      password: &String,
                      db_fread_mutex: &Arc<Mutex<File>>
                      ) -> Result<(), String> {

  let mut file_read = (*db_fread_mutex).lock().unwrap(); 
  file_read.seek(SeekFrom::Start(0)).unwrap(); 
  let reader: BufReader<&File> = BufReader::new(&*file_read);
  let mut cnt: u8 = 0;

  let mut x: Vec<String>;
  for line in reader.lines() {
    let cur_line: String = line.map_err(|_| "Error".to_string())?;
    x = cur_line.split(",").map(|e| e.to_string()).collect();
    if x.len() != 5 {
      return Err(format!("Error in 'db.txt' at line {}", cnt));
    }
    if x[0] == *username {
      if x[1] == *password {
        return Ok(());
      }
    }
    cnt += 1;
  }
  Err("Credentials not valid".to_string())
}

///////////// Allow slight one request exceeding ////////////////

fn verify_tokens(username: &String,
                 db_fread_mutex: &Arc<Mutex<File>>,
                 ) -> Result<(usize, String, String, String), String> {

  let n_repeat: usize = 12 - (*username).len();
  let username2: String = format!("{}{},", *username, " ".repeat(n_repeat));
  println!("username: {}|", username2);
  let username3: Vec<u8> = username2.into_bytes();
  let mut file = (*db_fread_mutex).lock().unwrap();
  file.seek(SeekFrom::Start(0)).unwrap();
  let reader: BufReader<&File> = BufReader::new(&*file);
  let mut cnt: usize = 0;

  for line in reader.lines() {
  
    let cur_string: String = line.map_err(|_| "Eror reading 'databases/db.txt'".to_string())?;
    println!("cur_string: {}", cur_string);
    let content: Vec<u8> = (cur_string.clone()).into_bytes();
    
    if content.starts_with(&username3) {
      let cur_vec: Vec<String> = cur_string
                                 .split(",")
                                 .map(|e| e.to_string())
                                 .collect();
      
      if cur_vec.len() != 5 {
        return Err(format!("Error in 'databases/db.txt' at line {}", cnt));
      }
      
      let now_tokens: u64 = cur_vec[3]
                            .parse::<u64>()
                            .map_err(|_| format!("Error while parsing current tokens for user: {}", cur_vec[0]))?;
      let max_tokens: u64 = cur_vec[4]
                            .parse::<u64>()
                            .map_err(|_| format!("Error while parsing max tokens for user: {}", cur_vec[0]))?;
      
      if now_tokens >= max_tokens {
        return Err("Not enough tokens".to_string());
      } else {

        println!("cur_vec: {:?}", cur_vec);

        return Ok((cnt, format!("{},{},{}", cur_vec[0].clone(), 
                    cur_vec[1].clone(), 
                    cur_vec[2].clone()), 
                cur_vec[3].clone(), cur_vec[4].clone()));
      }
    
    }
    cnt += 1;
  }
  Err("Could not locate the username".to_string())
}

fn incr_tokens(line_data: (usize, String, String, String),
               incr_value: u64,
               db_fwrite_mutex: &Arc<Mutex<File>>) -> Result<(), String> {
  
  let (index, credentials, cur_tokens, max_tokens) = line_data;
  println!("cur_tokens: {}", cur_tokens);
  let mut nb_used_tokens: u64 = cur_tokens.parse::<u64>()
                                      .map_err(|_| "Error parsing used tokens".to_string())?;
  nb_used_tokens += incr_value;
  let mut string_used_tokens: String = nb_used_tokens.to_string();
  let n_repeat: usize = 10 - string_used_tokens.len();
  let add_string: String = "0".repeat(n_repeat);
  
  string_used_tokens = format!("{}{}", add_string, string_used_tokens);
  let updated_line: String = format!("{},{},{}",
                                     credentials,
                                     string_used_tokens,
                                     max_tokens);

  const LINE_LEN: usize = 63;
  
  let mut file = (*db_fwrite_mutex).lock().unwrap();

  let offset: usize = index * LINE_LEN;
  file.seek(SeekFrom::Start(offset as u64))
      .map_err(|e| e.to_string())?;
  file.write_all(updated_line.as_bytes())
      .map_err(|e| e.to_string())?;
  Ok(())
}

/////////////////////////////////////////////////////////////////

//////// When user do a request ////////

fn do_the_req(username: &String,
              db_fwrite_mutex: &Arc<Mutex<File>>,
              db_fread_mutex: &Arc<Mutex<File>>) -> Result<(), String> {
  let (index, 
      credentials, 
      cur_tokens, 
      max_tokens) = verify_tokens(username,
                                  db_fread_mutex)?;
  let incr_value = 100;
  incr_tokens((index, credentials, cur_tokens, max_tokens), 
              incr_value,
              db_fwrite_mutex)?;
  Ok(())
}

///////////////////////////////////////

fn main() -> Result<(), String> {
  let listener: TcpListener = TcpListener::bind("127.0.0.1:8080")
      .unwrap();
  
  let thread_pool: ThreadPool = ThreadPool::new(15);
 
  //// mutexes creation for different files and different file operation ////

  let mut file = OpenOptions::new()
                      .create(true)
                      .append(true)
                      .open("databases/db.txt")
                      .map_err(|_| "Error opening to 'databases/db.txt'"
                          .to_string())?;

  let db_mutex_append: Arc<Mutex<File>> = Arc::new(
                                           Mutex::new(file)
                                         );

  file = OpenOptions::new()
                      .read(true)
                      .open("databases/db.txt")
                      .map_err(|_| "Error opening 'databases/db.txt'")?;
  
  let db_mutex_read: Arc<Mutex<File>> = Arc::new(
                                             Mutex::new(file)
                                           );

  file = OpenOptions::new()
                      .write(true)
                      .open("databases/db.txt")
                      .map_err(|_| "Error opening 'databases/db.txt'")?;
  
  let db_mutex_write: Arc<Mutex<File>> = Arc::new(
                                             Mutex::new(file)
                                           );

  //file = OpenOptions::new()
  //    .read(true)
  //    .open("/dev/urandom")
  //    .map_err(|_| "Error opening to 'dev/urandom'"
  //            .to_string())?;

  //let random_data_mutex_read: Arc<Mutex<File>> = Arc::new(
  //                                         Mutex::new(file)
  //                                       );

  ///////////////////////////////////////////////

  /////////////// Common HashSet creation /////////////////

  let mut prev_session_tokens: HashSet<[u8; 32]> = HashSet::new();

  file = OpenOptions::new()
      .read(true)
      .open("databases/session.txt")
      .map_err(|_| "Error opening to 'databases/session.txt'"
              .to_string())?;

  let cur_reader: BufReader<File> = BufReader::new(file);
  for line in cur_reader.lines() {
    let mut cur_val: [u8; 32] = [0u8; 32];
    let cur_line: String = line.unwrap().to_string();
    for i in 0..32 {
      cur_val[i] = u8::from_str_radix(&cur_line[i * 2..i * 2 + 2], 16)
                       .unwrap();
    }
    prev_session_tokens.insert(cur_val);
  }

  let session_tokens: Arc<Mutex<HashSet<[u8; 32]>>> = Arc::new(
                                                      Mutex::new(
                                                        prev_session_tokens
                                                      )
                                                    );

  ///////////////////////////////////////////////
  
  let running = Arc::new(AtomicBool::new(true));
  let r = running.clone();
 
  let session_tokens_for_sig: Arc<Mutex<HashSet<[u8; 32]>>> = session_tokens.clone();
  fn ending_func(x: &Arc<Mutex<HashSet<[u8; 32]>>>) {
    let session_read_mutex = (*x).lock().unwrap();
    let mut file = OpenOptions::new()
                      .write(true)
                      .open("databases/session.txt")
                      .unwrap();
    file.seek(SeekFrom::Start(0))
      .unwrap();
    for tkn in session_read_mutex.iter() {
      let mut cur_tkn: String = to_hex(tkn);
      cur_tkn.push('\n');
      file.write_all(cur_tkn.as_bytes())
        .unwrap();
    }
    println!("Ending lol");
  }

  ctrlc::set_handler(move || {
    println!("SIGINT received");
    ending_func(&session_tokens_for_sig);
    r.store(false, Ordering::SeqCst);
    std::process::exit(0);
  }).expect("Error setting ctrlc handler");

  ///////////////////////////////////////////////
    
  while running.load(Ordering::SeqCst) {
    for stream in listener.incoming() {
      let cur_stream: TcpStream = stream.unwrap();

      let clones = (
          db_mutex_append.clone(),
          db_mutex_read.clone(),
          session_tokens.clone(),
          db_mutex_write.clone(),
      );

      thread_pool.execute(move || {
          handle_request(cur_stream, clones.0, clones.1, 
                         clones.2, clones.3);
      });
    } 
  }
  Ok(())
}


