use super::*;

#[test]
fn paths_preserve_bytes() {
  let cases: &[(&[u8], &[u8])] = &[
    (b"book.pdf", b"book.pdf"),
    (b"../a b%.pdf", b"../a b%.pdf"),
    (b"/a\xff.pdf", b"/a\xff.pdf"),
    (b"file:///a%FF.pdf", b"/a\xff.pdf"),
    (b"file://localhost/a%20b.pdf", b"/a b.pdf"),
    (b"file:/a%2523+%23.pdf", b"/a%23+#.pdf"),
    (b"FILE://LOCALHOST/a.pdf", b"/a.pdf"),
    ("café.pdf".as_bytes(), "café.pdf".as_bytes()),
  ];
  for (input, expected) in cases {
    let reference = Reference::parse(OsStr::from_bytes(input)).unwrap();
    assert_eq!(reference.path.as_os_str().as_bytes(), *expected);
    assert_eq!(reference.operations, [] as [Operation; 0]);
  }
}

#[test]
fn ordered_targets_decode_once() {
  let reference = Reference::parse(OsStr::new(concat!(
    "book.pdf#page=12&nameddest=a%26b%23c%3Dd#page=1",
    "&structelem=%FF&comment=a+b&search=%22two%20words%22",
    "&highlight=1,2,3,4",
  )))
  .unwrap();
  assert_eq!(reference.path, PathBuf::from("book.pdf"));
  assert_eq!(
    reference.operations,
    vec![
      Operation::Page(NonZeroUsize::new(12).unwrap()),
      Operation::NamedDestination(b"a&b#c=d".to_vec()),
      Operation::Page(NonZeroUsize::new(1).unwrap()),
      Operation::Structure(vec![255]),
      Operation::Comment(b"a+b".to_vec()),
      Operation::Search(vec!["two".into(), "words".into()]),
      Operation::Highlight([1.0, 2.0, 3.0, 4.0]),
    ]
  );
}

#[test]
fn invalid_references_fail() {
  for input in [
    "",
    "file:relative.pdf",
    "file://papers/book.pdf",
    "file:///a.pdf?q=1",
    "file:///a b.pdf",
    "file:///café.pdf",
    "https://example.org/a.pdf",
    "file:///a%zz.pdf",
    "file:///a%0.pdf",
    "file:///a%.pdf",
    "file:///a%00.pdf",
    "a.pdf#",
    "a.pdf#page=0",
    "a.pdf#page=-1",
    "a.pdf#page=+1",
    "a.pdf#page=1.5",
    "a.pdf#page=9999999999999999999999999999999999999",
    "a.pdf#page=1&",
    "a.pdf#nameddest=",
    "a.pdf#comment=%00",
    "a.pdf#structelem=%zz",
    "a.pdf#zoom=100",
    "a.pdf#view=Fit",
    "a.pdf#viewrect=1,2,3,4",
    "a.pdf#ef=a.pdf",
    "a.pdf#fdf=https://example.org/a.fdf",
    "a.pdf#search=words",
    "a.pdf#search=%22%22",
    "a.pdf#search=%22%FF%22",
    "a.pdf#highlight=1,2,3",
    "a.pdf#highlight=2,1,3,4",
    "a.pdf#highlight=-1,2,3,4",
    "a.pdf#highlight=1,2,3,NaN",
    "a.pdf#highlight=1%2C2,3,4",
    "a.pdf#page=1:~:text=word",
  ] {
    assert!(Reference::parse(OsStr::new(input)).is_err(), "{input}");
  }
}

#[test]
fn resolution_keeps_targets_separate() {
  let reference = Reference::parse(OsStr::new("../book.pdf#page=2")).unwrap();
  assert_eq!(
    reference.resolve(Path::new("/launch")),
    Path::new("/launch/../book.pdf")
  );
  let absolute = Reference::parse(OsStr::new("/book.pdf#page=1")).unwrap();
  assert_eq!(absolute.resolve(Path::new("/launch")), Path::new("/book.pdf"));
}
