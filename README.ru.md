# one-brackets

[English](README.md) | **Русский**

Чтение и запись скобочного формата 1С:Предприятия: `{1,"текст",{вложенный,список}}`.

Формат используется по всей платформе 1С: файлы конфигурации, формы, настройки кластера
(`1cv8wsrv.lst`), журнал регистрации (`*.lgp`) и результат `ЗначениеВСтрокуВнутр()`.

- **Значения без потерь.** Строки в кавычках и значения без кавычек хранятся раздельно, поэтому
  текст, записанный платформой, записывается обратно побайтово. Проверено на файлах, записанных
  самой платформой. Пробелы между элементами не сохраняются: запись идёт в раскладке платформы,
  и `{ 1 , 2 }` становится `{1,2}`.
- **Любой источник.** Чтение из любого `std::io::Read`, потоково.
- **Любая глубина.** Разбор, запись, копирование, сравнение, хеширование и удаление обходятся
  без рекурсии, так что глубокая вложенность не переполняет стек. Отображение через serde
  рекурсивно, как и сам serde.
- **Понятные ошибки.** Ошибки разбора и доступа содержат строку, колонку и путь к узлу.
- **Редактирование.** Узлы можно менять на месте и записывать файл обратно.
- **serde.** Списки отображаются на ваши типы по порядку полей (фича `serde`).

## Использование

```toml
[dependencies]
one-brackets = "0.1"
```

Разбор, чтение и правка:

```rust
use one_brackets::Node;

fn main() -> one_brackets::Result<()> {
    let mut node: Node = "{2,c2f34349-b1e0-4060-8abf-3c59f38ac69f,\"Демо\"}".parse()?;

    assert_eq!(node.try_get(0)?.try_parse::<u32>()?, 2);
    assert!(node[1].as_guid().is_some());
    assert_eq!(node[2].as_quoted(), Some("Демо"));

    node[2] = Node::string("Переименовано");
    assert_eq!(node.to_string(), "{2,c2f34349-b1e0-4060-8abf-3c59f38ac69f,\"Переименовано\"}");

    let err = Node::try_from("{1,\n{2").unwrap_err();
    assert_eq!(
        err.to_string(),
        "unexpected end of input at line 2, column 3: list opened at line 2, column 1 is not closed",
    );
    Ok(())
}
```

Файлы читаются через `Node::try_from(Path::new("Form.txt"))` или `Node::from_reader(reader)`,
записываются через `node.write_to(writer)` или `node.to_string()`.

Тексты ошибок — на английском, как принято в экосистеме Rust.

### Отображение на свои типы

С `features = ["serde"]` элементы списка попадают в поля в порядке объявления.
`Rest` сохраняет элементы после последнего поля, поэтому неизвестные данные переживают
чтение и запись:

```rust
use one_brackets::Rest;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct Cluster {
    #[serde(with = "one_brackets::atom")]
    id: String,
    name: String,
    port: u16,
    #[serde(default)]
    rest: Rest,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let text = "{e75cc2cd-9060-434e-972f-e42cc4cba705,\"Локальный кластер\",1541,\"SRV1C\",0}";
    let cluster: Cluster = one_brackets::from_str(text)?;
    assert_eq!(cluster.port, 1541);
    assert_eq!(cluster.rest.len(), 2);
    assert_eq!(one_brackets::to_string(&cluster)?, text);
    Ok(())
}
```

`Counted<T>` читает распространённые списки вида `{количество,элемент1,элемент2,...}`.
Ошибки указывают на узел:

```text
expected u8, found value `300` at line 2, column 3 (path [1, 0, 0]): number too large to fit in target type
```

### Журнал регистрации и другие последовательности значений

`Values` потоково читает значения верхнего уровня через запятую, например записи журнала
регистрации, и держит в памяти только текущее:

```rust
use one_brackets::Values;

fn main() -> Result<(), one_brackets::ParseError> {
    let log = "1CV8LOG(ver 2.0)\r\n794d72bc-c31d-4245-b5a3-c765d53177ea\r\n\r\n{20260908233443,N},\r\n{20260908233445,U}";
    let mut values = Values::new(log.as_bytes());
    for _ in 0..3 {
        values.read_line()?;
    }
    for record in values {
        let record = record?;
        println!("{} at {}", record[0], record.position().unwrap());
    }
    Ok(())
}
```

## Примеры

- [`cluster`](examples/cluster/main.rs): реестр кластеров `1cv8wsrv.lst` через serde.
- [`v8log`](examples/v8log/main.rs): записи журнала регистрации, имена из словаря `1Cv8.lgf`.

```sh
cargo run --example v8log --features serde -- path/to/20260908000000.lgp
```

## Утилита командной строки

[`one-brackets-cli`](cli/README.ru.md) без потерь переводит скобочные файлы в JSON, YAML и XML
и обратно, проверяет их и выводит узлы по пути:

```sh
cargo install one-brackets-cli
one-brackets export Form.txt -o Form.yaml
one-brackets import Form.yaml -o Form.txt
```

## Производительность

При разборе каждое значение — отдельный `String`, каждый список — отдельный `Vec`, поэтому
на больших файлах всё упирается в аллокатор памяти. Библиотека его не выбирает; приложение
может перейти на [mimalloc](https://crates.io/crates/mimalloc), он работает в Windows, Linux
и macOS:

```toml
[dependencies]
mimalloc = "0.1"
```

```rust,ignore
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
```

Лучший из 7 прогонов, 3 МБ форм и 4 МБ записей журнала регистрации:

| ОС | Аллокатор | Формы | Журнал |
|---|---|---|---|
| Windows | системный | 85 мс | 67 мс |
| Windows | mimalloc | 32 мс (×2,7) | 44 мс (×1,5) |
| Linux (glibc 2.39) | системный | 48 мс | 49 мс |
| Linux (glibc 2.39) | mimalloc | 32 мс (×1,5) | 40 мс (×1,2) |
| Linux (glibc 2.39) | jemalloc | 36 мс (×1,3) | 39 мс (×1,25) |

Больше всего выигрыш в Windows: её системный аллокатор самый медленный из перечисленных.
В Linux jemalloc не уступает mimalloc, но не собирается под Windows MSVC.
Для журналов берите `Values` вместо разбора одного огромного списка и обрабатывайте файлы
параллельно: журнал разбит на файлы по периодам. `cargo bench` измеряет скорость чтения
и записи.

## Минимальная версия Rust

1.85 (edition 2024).

## Лицензия

На ваш выбор: [Apache License, Version 2.0](LICENSE-APACHE) или
[MIT license](LICENSE-MIT).

Если вы явно не укажете иное, любой вклад, намеренно отправленный вами для включения в этот
крейт, как это определено в лицензии Apache-2.0, распространяется под двойной лицензией,
указанной выше, без каких-либо дополнительных условий.
