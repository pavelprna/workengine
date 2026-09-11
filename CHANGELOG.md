# Changelog

All notable changes to Workengine are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Release Please generates released sections from Conventional Commits; humans add
context only when the generated notes would be misleading or incomplete.

## [0.2.0](https://github.com/pavelprna/workengine/compare/v0.1.0...v0.2.0) (2026-09-11)


### Added

* **application:** add execution observation ports ([f3cc63f](https://github.com/pavelprna/workengine/commit/f3cc63f8749e8003d38c7933c98e1c8787e227cc))
* **application:** add project queues and integration boundaries ([1da426f](https://github.com/pavelprna/workengine/commit/1da426fd0cf0bc27503ea55b1070a0817f39984a))
* **cli:** add lease-protected operator launch ([3106047](https://github.com/pavelprna/workengine/commit/3106047c5f3b63044a25f8222757a44e59e3fffc))
* **cli:** add local Work intake ([f0dad8e](https://github.com/pavelprna/workengine/commit/f0dad8eee55c0bbd5e2c1a74ef06f8d5a52a2c4a))
* **cli:** add v0.4 execution ledger UI ([cb9fecc](https://github.com/pavelprna/workengine/commit/cb9fecce4eddb2f0cb9264ec6f32173309b34be9))
* **cli:** add v0.5 live operator daemon ([9f1af46](https://github.com/pavelprna/workengine/commit/9f1af46816885ce92a196f32c5169817685d0ddd))
* **cli:** add Work queue interface ([2f45b32](https://github.com/pavelprna/workengine/commit/2f45b327236c0912eae80f8ce144f2851d50ff0f))
* **cli:** expose explicit Work launch in browser ([37e233d](https://github.com/pavelprna/workengine/commit/37e233d3e37782434de99fc4dd4fbd8d740b27e9))
* **cli:** serve execution observation flow ([8b64151](https://github.com/pavelprna/workengine/commit/8b64151f6365ede2d58f3581715a324289d1a628))
* **domain:** add durable execution vocabulary ([66d63cf](https://github.com/pavelprna/workengine/commit/66d63cf6a6d7a4391214d70d075439979269c816))
* **domain:** add runtime kind to execution spec ([2e32ce8](https://github.com/pavelprna/workengine/commit/2e32ce804b6c21a2439f65a511e123a4c9052b94))
* **spec:** expose execution observation API ([ad0ee27](https://github.com/pavelprna/workengine/commit/ad0ee27a30c4cea76f004f159f9a3b5e11c9a88a))
* **store:** persist execution observations ([831d2b4](https://github.com/pavelprna/workengine/commit/831d2b45718d8d840f59e0b9374f292ad442ef6e))
* **worker:** enforce v0.6 runtime policy ([bb54f55](https://github.com/pavelprna/workengine/commit/bb54f556f42adf87637fa6d6ccacae421d15529d))
* **worker:** record payload-free process observations ([3d19ed0](https://github.com/pavelprna/workengine/commit/3d19ed0999c9e8c686cc08df57a609383a14f5d4))


### Documentation

* **adr:** define execution and attempt identity ([e84343c](https://github.com/pavelprna/workengine/commit/e84343cbfd781cc6ee6cfcc1b6c1b5645e7d3333))
* **adr:** document v0.3 operator launch ([253a653](https://github.com/pavelprna/workengine/commit/253a6536c399915f8d2c68cae3cdff95dfa1f310))
* **adr:** record observable execution projection ([aabae8b](https://github.com/pavelprna/workengine/commit/aabae8bc568f08a1460e55e61ffef2a5aefbb36d))
* **adr:** record v0.6 worker policy ([79dcc16](https://github.com/pavelprna/workengine/commit/79dcc16fa341d21a6ee509e9f5047ea776735a44))
* **spec:** document v0.4 observation surface ([505d036](https://github.com/pavelprna/workengine/commit/505d03670ef3f866f5ef2eb0f52b45d23bf062b3))

## 0.1.0 (2026-09-11)


### chore

* establish initial pre-1.0 release ([dfc8801](https://github.com/pavelprna/workengine/commit/dfc88015fc3bf4f25f91ef8337dfe66924001538))


### Added

* **application:** add create next start complete park ports ([949a9ed](https://github.com/pavelprna/workengine/commit/949a9eda87f643e143eb7bcc8019e1bca2b2985b))
* **application:** classify channel errors as retry fail or park ([24dcb0f](https://github.com/pavelprna/workengine/commit/24dcb0f85084c4d3bc2ddda293fc3ddb8d503054))
* **cli:** add Web observer application shell ([00abf53](https://github.com/pavelprna/workengine/commit/00abf539bb42c8a1f45bae8635a7fc8130224195))
* **cli:** expose create next start complete park ([e1db7bf](https://github.com/pavelprna/workengine/commit/e1db7bfc7ad527f11412858256c5c7a6696c8720))
* **cli:** expose help and version ([d001e37](https://github.com/pavelprna/workengine/commit/d001e37c00158cb1670b2aacc3d3fabf5dee6a81))
* **cli:** init tracing and print version with git sha ([119cc6b](https://github.com/pavelprna/workengine/commit/119cc6b72c58dbf0f7897717ef2cb4cf6dc8ce00))
* **cli:** load worker profiles from config ([9a89e95](https://github.com/pavelprna/workengine/commit/9a89e95b35c9bae0dbe595edaad7532beef42cde))
* **cli:** serve read-only local observer ([a63d684](https://github.com/pavelprna/workengine/commit/a63d684cd74841018dcfe4744d3b5995175abeb3))
* **domain:** add work identity fsm and closed outcomes ([33d0e28](https://github.com/pavelprna/workengine/commit/33d0e289143b0e6e20acccebf96c416ca4d674c8))
* **domain:** allow complete from parked leftover ([1dd7ab7](https://github.com/pavelprna/workengine/commit/1dd7ab7d7a3a953b759e49824c14dcfd3c370e03))
* **domain:** stamp event time and reject terminal workspace bind ([dcd7f86](https://github.com/pavelprna/workengine/commit/dcd7f86fc0537afeee62bd701a787392b7ede632))
* **store:** lock the data directory for the CLI process ([4de2665](https://github.com/pavelprna/workengine/commit/4de2665284f125da5c7fed7c2f1e592091ea6de0))
* **store:** migrate identity schema and refuse unknown versions ([727b8d3](https://github.com/pavelprna/workengine/commit/727b8d3bccb9b5ebe8706a058120cc32d325f034))
* **store:** persist state and events atomically ([9ca5f8a](https://github.com/pavelprna/workengine/commit/9ca5f8adb53677533c754b5fbe9222c437f34193))
* **worker:** add a generic process runner ([e67f3d4](https://github.com/pavelprna/workengine/commit/e67f3d4cdfae0477ded557f3eb2942c305c7de72))
* **worker:** emit one camelCase JSON stream on stderr ([830bd4f](https://github.com/pavelprna/workengine/commit/830bd4f556ca05c4a45fb46ef7faf0c8225ee740))
* **worker:** harden sandboxed execution ([3c891e8](https://github.com/pavelprna/workengine/commit/3c891e8a17c53740d745bc088a4ef93b6e4551a7))
* **worker:** spawn stub with budget timeout ([9242298](https://github.com/pavelprna/workengine/commit/9242298464049c18fe1831d35124b7a2ee14fe67))
* **worker:** wrap subprocess output in one JSON stream ([9ee7d3b](https://github.com/pavelprna/workengine/commit/9ee7d3bf39d6d3a439ec97568650318fcd740e75))
* **workspace:** isolate a directory per work id ([1226fae](https://github.com/pavelprna/workengine/commit/1226faef2f6e7304a70672dca64de45706bca693))
* **workspace:** persist typed memory and skip duplicate lines ([765fb67](https://github.com/pavelprna/workengine/commit/765fb67e7ad0a35b27961f6e735e63d900d2dbcd))


### Fixed

* **application:** resume leftover Work without a second start ([10b0241](https://github.com/pavelprna/workengine/commit/10b02418a87412c5f328c06b83bafeb96fd5aac8))
* **arch:** scan domain names outside the domain crate ([fb0602a](https://github.com/pavelprna/workengine/commit/fb0602a4c10915060a7fee713e004e5b733fc0ae))
* **cli:** recover parked leftover and emit classified timeout exits ([39fb757](https://github.com/pavelprna/workengine/commit/39fb757ba08e7f2def86f122decec4b8f3ae4c7f))
* support inherited workspace versions ([dca8dcc](https://github.com/pavelprna/workengine/commit/dca8dcc7072463617e23eb3837a344fdebc151a7))
* support workspace-root release version ([22c3aac](https://github.com/pavelprna/workengine/commit/22c3aac8cae23ba0d9989403a3ab2c1b5ec71e26))
* **worker:** copy outcome from a file instead of a shell string ([30da34c](https://github.com/pavelprna/workengine/commit/30da34cd9373b1740109d05d803c09986b110b14))
* **worker:** return timed_out and budget_exceeded kinds ([61c88a3](https://github.com/pavelprna/workengine/commit/61c88a362beb53fe7d359ff000451dc10a0c9c8f))


### Documentation

* add the agent entry ([15f4a84](https://github.com/pavelprna/workengine/commit/15f4a84f99df7c67388a898d709a656724d10592))
* add the document map ([841ced7](https://github.com/pavelprna/workengine/commit/841ced7acb8a14531cbaed0f5b2c146b48820807))
* **adr:** choose Rust hexagonal crates ([b039681](https://github.com/pavelprna/workengine/commit/b039681ac02bbcc265c4c975a94e8b272802dd88))
* **adr:** keep Work status internal ([38437df](https://github.com/pavelprna/workengine/commit/38437dfa74d958c5f31d3c439434f58ea005c436))
* **adr:** record architecture decisions ([dd4d7d3](https://github.com/pavelprna/workengine/commit/dd4d7d3523a4d0024dc4e95a7110a5777425e380))
* **adr:** record worker profile as configuration ([3124b40](https://github.com/pavelprna/workengine/commit/3124b4067e9105c870ecf55a416a2ab9cc6f9bcf))
* **adr:** run Worker as an OS process ([d4a4566](https://github.com/pavelprna/workengine/commit/d4a4566ec1996bee844248ac9ddd03dedb5f6457))
* **arch:** add fitness rows for park, memory, and stream ([87a551b](https://github.com/pavelprna/workengine/commit/87a551b2ccc0baaa92dfb4c6bf2483b2da4d655a))
* **arch:** add process adapter and fitness F24 F25 ([1a48ea5](https://github.com/pavelprna/workengine/commit/1a48ea5008cc44d5a154077c95b584ee83cb3330))
* **arch:** declare fitness functions ([c99f826](https://github.com/pavelprna/workengine/commit/c99f826a3e8dc68cce5181ebc1f52931ae853f79))
* **arch:** describe Clock as create timestamps, not hang detection ([0774f3c](https://github.com/pavelprna/workengine/commit/0774f3c8f8ad93fbbac0f247ce2391ca1b1e779a))
* **arch:** describe crate layers ([a81d26a](https://github.com/pavelprna/workengine/commit/a81d26aeda1318a72d461604f8dc2047dbb337bc))
* **arch:** map ports to first adapters ([0703878](https://github.com/pavelprna/workengine/commit/0703878ee58785abd7034449c8130d2cecdcdda0))
* document config and checkout flags ([58f11f9](https://github.com/pavelprna/workengine/commit/58f11f9d43fdf9f16d7b13ba8ec8fca14c4e0a37))
* document contribution process ([fc0ed3f](https://github.com/pavelprna/workengine/commit/fc0ed3f7ffedc8322abd1798ed75a796e2333a0f))
* draft SemVer and exit-code charter ([5546cf1](https://github.com/pavelprna/workengine/commit/5546cf1468b2212779461cb4d4e1f2f068b1f231))
* freeze first-slice exit codes ([9f61855](https://github.com/pavelprna/workengine/commit/9f61855ce4fe291bdbb01bc013a5ba4cb288fa71))
* record the spawn threat model ([965d84f](https://github.com/pavelprna/workengine/commit/965d84f7491be732591c40e93390d87712f97f6e))
* rewrite README as the product front door ([736cc37](https://github.com/pavelprna/workengine/commit/736cc3788156b689c345314bec763ed1220396d1))
* **spec:** align roadmap with current guarantees ([a1105dc](https://github.com/pavelprna/workengine/commit/a1105dc3e68fce9ff853140439d70f5a07cd5d94))
* **spec:** clarify local Work creation ([52ac035](https://github.com/pavelprna/workengine/commit/52ac035952f4f21b8da44d26e72d04ff6067f519))
* **spec:** close remaining Workflow MUST gaps ([03c28d8](https://github.com/pavelprna/workengine/commit/03c28d848a425fc4a0300b926e3f93e9058d4092))
* **spec:** define Work ([83ff9da](https://github.com/pavelprna/workengine/commit/83ff9da660e767270710efb12fbd393d7c527130))
* **spec:** define Worker ([35795bf](https://github.com/pavelprna/workengine/commit/35795bfd6d4d3d3deac97bd9b0b99745fba7ada6))
* **spec:** define Workflow ([fc50ecd](https://github.com/pavelprna/workengine/commit/fc50ecdec0f59b2c5d5a6ce5e2e86fa248a933bd))
* **spec:** define Workspace ([ce56562](https://github.com/pavelprna/workengine/commit/ce56562af00ed12a40e92b17b14a32779fcf0547))
* **spec:** mark channel checkout and secret-ref MUST rows ([4ee17a4](https://github.com/pavelprna/workengine/commit/4ee17a406987a9ad5246e3b01a9569190f6971e6))
* **spec:** mark first-slice MUST rows that tests already cover ([bae00b9](https://github.com/pavelprna/workengine/commit/bae00b9586380a5094af4d241dec0b387c97fb34))
* **spec:** mark stream and domain-name MUST rows tested ([4429248](https://github.com/pavelprna/workengine/commit/442924860e8cb6c97db0c5a44851091ad43b6a71))
* **spec:** record first-slice create start and resume ([f2e122d](https://github.com/pavelprna/workengine/commit/f2e122d93035f72109958f4ea7f273b798841f84))
* **spec:** record first-slice hang as the runner deadline ([82e9dab](https://github.com/pavelprna/workengine/commit/82e9dab6bb0353a80f97ffa19382a2deac1c8f73))
* **spec:** require workspace cumulative memory ([80e12ec](https://github.com/pavelprna/workengine/commit/80e12ece23a7de58259cfdd5792f6915bb1a67a3))

## [Unreleased]

### Added

- Release automation and the product roadmap.
