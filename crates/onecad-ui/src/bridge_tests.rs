use super::*;
#[test]
fn closed_queue_rejects_racing_submit_and_releases_permit() {
    let permits = Arc::new(Semaphore::new(ADMISSION));
    let (commands, mut queue) = async_mpsc::channel(ADMISSION);
    admit(&commands, &permits, Command::Save).unwrap();
    queue.close();
    assert!(admit(&commands, &permits, Command::Save).is_err());
    assert_eq!(permits.available_permits(), ADMISSION - 1);
    drop(queue.try_recv().unwrap());
    assert_eq!(permits.available_permits(), ADMISSION);
}
#[test]
fn actual_admission_keeps_capacity_reserved_until_terminal_delivery() {
    let permits = Arc::new(Semaphore::new(ADMISSION));
    let (commands, mut queue) = async_mpsc::channel(ADMISSION);
    for _ in 0..ADMISSION {
        admit(&commands, &permits, Command::Save).unwrap();
    }
    assert!(admit(&commands, &permits, Command::Save).is_err());
    let (sender, receiver) = mpsc::channel();
    for _ in 0..ADMISSION {
        let envelope = queue.try_recv().unwrap();
        sender
            .send(Message::Terminal {
                command: "Save",
                tag: None,
                result: Ok(Default::default()),
                _permit: envelope.permit,
            })
            .unwrap();
    }
    assert!(admit(&commands, &permits, Command::Save).is_err());
    drop(receiver.recv().unwrap());
    admit(&commands, &permits, Command::Save).unwrap();
    assert!(admit(&commands, &permits, Command::Save).is_err());
}
