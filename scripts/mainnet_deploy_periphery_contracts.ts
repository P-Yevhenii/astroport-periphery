import "dotenv/config";
import {
  deployContract,
  executeContract,
  newClient,
  transfer_ownership_to_multisig,
  readArtifact,
  writeArtifact,
  Client,
} from "./helpers/helpers.js";
import { mainnet, Config } from "./deploy_configs.js";
import { join } from "path";

const ASTRO_MULTISIG = "terra1c7m6j8ya58a2fkkptn8fgudx8sqjqvc8azq0ex";

const LOCKDROP_INCENTIVES = 75_000_000_000000; // 7.5 Million = 7.5%
const AIRDROP_INCENTIVES = 25_000_000_000000; // 2.5 Million = 2.5%
const AUCTION_INCENTIVES = 10_000_000_000000; // 1.0 Million = 1%
// LOCKDROP INCENTIVES
const LUNA_UST_ASTRO_INCENTIVES = 21_750_000_000000;
const LUNA_BLUNA_ASTRO_INCENTIVES = 17_250_000_000000;
const ANC_UST_ASTRO_INCENTIVES = 14_250_000_000000;
const MIR_UST_ASTRO_INCENTIVES = 6_750_000_000000;
const ORION_UST_ASTRO_INCENTIVES = 1_500_000_000000;
const STT_UST_ASTRO_INCENTIVES = 3_750_000_000000;
const VKR_UST_ASTRO_INCENTIVES = 2_250_000_000000;
const MINE_UST_ASTRO_INCENTIVES = 3_000_000_000000;
const PSI_UST_ASTRO_INCENTIVES = 2_250_000_000000;
const APOLLO_UST_ASTRO_INCENTIVES = 2_250_000_000000;

const ARTIFACTS_PATH = "../artifacts";

async function main() {
  let CONFIGURATION: Config = mainnet;

  // terra, wallet
  const { terra, wallet } = newClient();
  console.log(
    `chainID: ${terra.config.chainID} wallet: ${wallet.key.accAddress}`
  );

  let network = readArtifact(terra.config.chainID);
  terra.config.chainID = terra.config.chainID;
  console.log("network:", network);

  // Terminate if not mainnet
  if (terra.config.chainID != "columbus-5") {
    console.log("Network must be Col-5");
    return;
  }

  // ASTRO Token addresss should be set
  if (!network.astro_token_address) {
    console.log(
      `Please set the ASTRO Token address in the deploy config before running this script...`
    );
    return;
  }

  /*************************************** DEPLOYMENT :: LOCKDROP CONTRACT  *****************************************/
  /*************************************** DEPLOYMENT :: LOCKDROP CONTRACT  *****************************************/
  /*************************************** DEPLOYMENT :: LOCKDROP CONTRACT  *****************************************/

  if (!network.lockdrop_address) {
    console.log(`${terra.config.chainID} :: Deploying Lockdrop Contract`);
    CONFIGURATION.lockdrop_InitMsg.config.owner = wallet.key.accAddress;
    console.log(CONFIGURATION.lockdrop_InitMsg);
    network.lockdrop_address = await deployContract(
      terra,
      wallet,
      join(ARTIFACTS_PATH, "astroport_lockdrop.wasm"),
      CONFIGURATION.lockdrop_InitMsg.config,
      "ASTROPORT Launch : Phase 1  -::- Lockdrop -::- Liquidity Migration"
    );
    writeArtifact(network, terra.config.chainID);
    console.log(
      `${terra.config.chainID} :: Lockdrop Contract Address : ${network.lockdrop_address} \n`
    );
  }

  /*************************************** DEPLOYMENT :: AIRDROP CONTRACT  *****************************************/
  /*************************************** DEPLOYMENT :: AIRDROP CONTRACT  *****************************************/
  /*************************************** DEPLOYMENT :: AIRDROP CONTRACT  *****************************************/

  if (!network.airdrop_address) {
    console.log(`${terra.config.chainID} :: Deploying Airdrop Contract`);
    // Set configuration
    CONFIGURATION.airdrop_InitMsg.config.owner = wallet.key.accAddress;
    CONFIGURATION.airdrop_InitMsg.config.merkle_roots = [
      "3307f438555508fb589a9f481e0de4b7366f5e7f993a41c80c374a80b0acf04e",
      "daf4daabcf252cd8654cdcd5d5e11827fd144c6f658bc3dcdd5f2397926a2dd5",
      "73a387947dd40d47211509dba94dcf457a3f7710dfa056d7e238d7f8dbcafbb1",
    ];
    CONFIGURATION.airdrop_InitMsg.config.astro_token_address =
      network.astro_token_address;
    // deploy airdrop contract
    console.log(CONFIGURATION.airdrop_InitMsg);
    network.airdrop_address = await deployContract(
      terra,
      wallet,
      join(ARTIFACTS_PATH, "astroport_airdrop.wasm"),
      CONFIGURATION.airdrop_InitMsg.config,
      "ASTROPORT Launch -::- ASTRO Airdrop"
    );
    console.log(
      `${terra.config.chainID} :: Airdrop Contract Address : ${network.airdrop_address} \n`
    );
    writeArtifact(network, terra.config.chainID);
  }

  /*************************************** DEPLOYMENT :: AUCTION CONTRACT  *****************************************/
  /*************************************** DEPLOYMENT :: AUCTION CONTRACT  *****************************************/
  /*************************************** DEPLOYMENT :: AUCTION CONTRACT  *****************************************/

  if (!network.auction_address) {
    console.log(`${terra.config.chainID} :: Deploying Auction Contract`);
    // Set configuration
    CONFIGURATION.auction_InitMsg.config.owner = ASTRO_MULTISIG;
    CONFIGURATION.auction_InitMsg.config.astro_token_address =
      network.astro_token_address;
    CONFIGURATION.auction_InitMsg.config.airdrop_contract_address =
      network.airdrop_address;
    CONFIGURATION.auction_InitMsg.config.lockdrop_contract_address =
      network.lockdrop_address;
    // deploy auction contract
    console.log(CONFIGURATION.auction_InitMsg);
    network.auction_address = await deployContract(
      terra,
      wallet,
      join(ARTIFACTS_PATH, "astroport_auction.wasm"),
      CONFIGURATION.auction_InitMsg.config,
      "ASTROPORT Launch -::- Auction -::- ASTRO-UST LP Pool Bootstrapping"
    );
    console.log(
      `${terra.config.chainID} :: Auction Contract Address : ${network.auction_address} \n`
    );
    network.auction_multisig_made_owner = true;
    writeArtifact(network, terra.config.chainID);
  }

  //  UpdateConfig :: SET ASTRO Token and Auction Contract in Lockdrop
  if (!network.lockdrop_astro_token_set && !network.auction_set_in_lockdrop) {
    console.log(
      `${terra.config.chainID} :: Setting ASTRO Token for Lockdrop...`
    );
    let tx = await executeContract(
      terra,
      wallet,
      network.lockdrop_address,
      {
        update_config: {
          new_config: {
            owner: undefined,
            astro_token_address: network.astro_token_address,
            auction_contract_address: network.auction_address,
            generator_address: undefined,
          },
        },
      },
      [],
      "ASTROPORT Launch -::-  Phase 1 -::-  Lockdrop -::-  UpdateConfig -::- Set ASTRO & Auction addresses"
    );
    console.log(
      `Lockdrop :: ASTRO Token & Auction contract set successfully set ${tx.txhash}\n`
    );
    network.lockdrop_astro_token_set = true;
    network.auction_set_in_lockdrop = true;
    writeArtifact(network, terra.config.chainID);
  }

  // UpdateConfig :: Set Auction address & update owner in airdrop
  if (!network.auction_set_in_airdrop) {
    // update Config Tx
    let out = await executeContract(
      terra,
      wallet,
      network.airdrop_address,
      {
        update_config: {
          owner: ASTRO_MULTISIG,
          auction_contract_address: network.auction_address,
          merkle_roots: undefined,
          from_timestamp: undefined,
          to_timestamp: undefined,
        },
      },
      [],
      "ASTROPORT Launch -::-  Phase 1 -::-  Airdrop -::-  UpdateConfig -::- Set Auction address, update owner "
    );
    console.log(
      `${terra.config.chainID} :: Setting auction contract address in ASTRO Airdrop contract,  ${out.txhash}`
    );
    network.auction_set_in_airdrop = true;
    network.airdrop_multisig_made_owner = true;
    writeArtifact(network, terra.config.chainID);
  }

  // ASTRO::Send::Lockdrop::IncreaseAstroIncentives:: Transfer ASTRO to Lockdrop and set total incentives
  if (!network.lockdrop_astro_token_transferred) {
    let transfer_msg = {
      send: {
        contract: network.lockdrop_address,
        amount: String(LOCKDROP_INCENTIVES),
        msg: Buffer.from(
          JSON.stringify({ increase_astro_incentives: {} })
        ).toString("base64"),
      },
    };
    let increase_astro_incentives = await executeContract(
      terra,
      wallet,
      network.astro_token_address,
      transfer_msg,
      [],
      "ASTROPORT Launch -::-  Phase 1 -::-  Lockdrop -::- Transfer ASTRO to Lockdrop for Incentives"
    );
    console.log(
      `${terra.config.chainID} :: Transferring ASTRO Token and setting incentives in Lockdrop... ${increase_astro_incentives.txhash}`
    );
    network.lockdrop_astro_token_transferred = true;
    writeArtifact(network, terra.config.chainID);
  }

  // ASTRO::Send::Airdrop::IncreaseAstroIncentives:: Transfer ASTRO to Airdrop
  if (!network.airdrop_astro_token_transferred) {
    // transfer ASTRO Tx
    let tx = await executeContract(
      terra,
      wallet,
      network.astro_token_address,
      {
        send: {
          contract: network.airdrop_address,
          amount: String(AIRDROP_INCENTIVES),
          msg: Buffer.from(
            JSON.stringify({ increase_astro_incentives: {} })
          ).toString("base64"),
        },
      },
      [],
      "ASTROPORT Launch -::-  Phase 1 -::-  Airdrop -::- Transfer ASTRO to Airdrop for Incentives"
    );
    console.log(
      `${terra.config.chainID} :: Transferring ASTRO Token and setting incentives in Airdrop... ${tx.txhash}`
    );
    network.airdrop_astro_token_transferred = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Set Auction incentives
  if (!network.auction_astro_token_transferred) {
    // transfer ASTRO Tx
    let msg = {
      send: {
        contract: network.auction_address,
        amount: String(AUCTION_INCENTIVES),
        msg: Buffer.from(
          JSON.stringify({ increase_astro_incentives: {} })
        ).toString("base64"),
      },
    };
    let out = await executeContract(
      terra,
      wallet,
      network.astro_token_address,
      msg,
      [],
      "ASTROPORT Launch -::-  Phase 1 -::-  Auction -::- Transfer ASTRO to Auction for Incentives"
    );
    console.log(
      `${terra.config.chainID} :: Transferring ASTRO Token and setting incentives in Auction... ${out.txhash}`
    );
    network.auction_astro_token_transferred = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Lockdrop -::- Initialize LUNA-UST Pool
  if (!network.luna_ust_lockdrop_pool_initialized) {
    let luna_ust_init_msg = {
      initialize_pool: {
        terraswap_lp_token: network.luna_ust_terraswap_lp_token_address,
        incentives_share: LUNA_UST_ASTRO_INCENTIVES,
      },
    };
    console.log(
      `${terra.config.chainID} :: Initializing LUNA-UST LP Token Pool in Lockdrop...`
    );
    let luna_ust_pool_init = await executeContract(
      terra,
      wallet,
      network.lockdrop_address,
      luna_ust_init_msg,
      [],
      "Lockdrop -::- Initialize LUNA-UST Pool"
    );
    console.log(luna_ust_pool_init.txhash);
    console.log(
      `Lockdrop :: Luna-ust Pool successfully initialized with Lockdrop \n`
    );
    network.luna_ust_lockdrop_pool_initialized = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Initialize LUNA-BLUNA Pool in Lockdrop
  if (!network.bluna_luna_lockdrop_pool_initialized) {
    let bluna_luna_init_msg = {
      initialize_pool: {
        terraswap_lp_token: network.bluna_luna_terraswap_lp_token_address,
        incentives_share: LUNA_BLUNA_ASTRO_INCENTIVES,
      },
    };

    console.log(
      `${terra.config.chainID} :: Lockdrop -::- Initialize LUNA-BLUNA LP Pool...`
    );
    let bluna_luna_pool_init = await executeContract(
      terra,
      wallet,
      network.lockdrop_address,
      bluna_luna_init_msg,
      [],
      "Lockdrop -::- Initialize LUNA-BLUNA LP Pool"
    );
    console.log(bluna_luna_pool_init.txhash);
    console.log(
      `Lockdrop :: LUNA-BLUNA Pool successfully initialized with Lockdrop \n`
    );
    network.bluna_luna_lockdrop_pool_initialized = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Initialize ANC-UST Pool in Lockdrop
  if (!network.anc_ust_lockdrop_pool_initialized) {
    let anc_ust_init_msg = {
      initialize_pool: {
        terraswap_lp_token: network.anc_ust_terraswap_lp_token_address,
        incentives_share: ANC_UST_ASTRO_INCENTIVES,
      },
    };
    console.log(
      `${terra.config.chainID} :: Lockdrop -::- Initialize ANC-UST LP Pool...`
    );
    let anc_ust_pool_init = await executeContract(
      terra,
      wallet,
      network.lockdrop_address,
      anc_ust_init_msg,
      [],
      "Lockdrop -::- Initialize ANC-UST LP Pool"
    );
    console.log(anc_ust_pool_init.txhash);
    console.log(
      `Lockdrop :: ANC-UST Pool successfully initialized with Lockdrop \n`
    );
    network.anc_ust_lockdrop_pool_initialized = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Initialize MIR-UST Pool in Lockdrop
  if (!network.mir_ust_lockdrop_pool_initialized) {
    let mir_ust_init_msg = {
      initialize_pool: {
        terraswap_lp_token: network.mir_ust_terraswap_lp_token_address,
        incentives_share: MIR_UST_ASTRO_INCENTIVES,
      },
    };
    console.log(
      `${terra.config.chainID} :: Lockdrop -::- Initialize MIR-UST LP Pool...`
    );
    let mir_ust_pool_init = await executeContract(
      terra,
      wallet,
      network.lockdrop_address,
      mir_ust_init_msg,
      [],
      "Lockdrop -::- Initialize MIR-UST LP Pool"
    );
    console.log(mir_ust_pool_init.txhash);
    console.log(
      `Lockdrop :: MIR-UST Pool successfully initialized with Lockdrop \n`
    );
    network.mir_ust_lockdrop_pool_initialized = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Initialize ORION-UST Pool in Lockdrop
  if (!network.orion_ust_lockdrop_pool_initialized) {
    let orion_ust_init_msg = {
      initialize_pool: {
        terraswap_lp_token: network.orion_ust_terraswap_lp_token_address,
        incentives_share: ORION_UST_ASTRO_INCENTIVES,
      },
    };
    console.log(
      `${terra.config.chainID} :: Lockdrop -::- Initialize ORION-UST LP Pool...`
    );
    let orion_ust_pool_init = await executeContract(
      terra,
      wallet,
      network.lockdrop_address,
      orion_ust_init_msg,
      [],
      "Lockdrop -::- Initialize ORION-UST LP Pool"
    );
    console.log(orion_ust_pool_init.txhash);
    console.log(
      `Lockdrop :: ORION-UST Pool successfully initialized with Lockdrop \n`
    );
    network.orion_ust_lockdrop_pool_initialized = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Initialize STT-UST Pool in Lockdrop
  if (!network.stt_ust_lockdrop_pool_initialized) {
    let stt_ust_init_msg = {
      initialize_pool: {
        terraswap_lp_token: network.stt_ust_terraswap_lp_token_address,
        incentives_share: STT_UST_ASTRO_INCENTIVES,
      },
    };
    console.log(
      `${terra.config.chainID} :: Lockdrop -::- Initialize STT-UST LP Pool...`
    );
    let stt_ust_pool_init = await executeContract(
      terra,
      wallet,
      network.lockdrop_address,
      stt_ust_init_msg,
      [],
      "Lockdrop -::- Initialize STT-UST LP Pool"
    );
    console.log(stt_ust_pool_init.txhash);
    console.log(
      `Lockdrop :: STT-UST Pool successfully initialized with Lockdrop \n`
    );
    network.stt_ust_lockdrop_pool_initialized = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Initialize VKR-UST Pool in Lockdrop
  if (!network.vkr_ust_lockdrop_pool_initialized) {
    let vkr_ust_init_msg = {
      initialize_pool: {
        terraswap_lp_token: network.vkr_ust_terraswap_lp_token_address,
        incentives_share: VKR_UST_ASTRO_INCENTIVES,
      },
    };

    console.log(
      `${terra.config.chainID} :: Lockdrop -::- Initialize VKR-UST LP Pool...`
    );
    let vkr_ust_pool_init = await executeContract(
      terra,
      wallet,
      network.lockdrop_address,
      vkr_ust_init_msg,
      [],
      "Lockdrop -::- Initialize VKR-UST LP Pool"
    );
    console.log(vkr_ust_pool_init.txhash);
    console.log(
      `Lockdrop :: VKR-UST Pool successfully initialized with Lockdrop \n`
    );
    network.vkr_ust_lockdrop_pool_initialized = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Initialize MINE-UST Pool in Lockdrop
  if (!network.mine_ust_lockdrop_pool_initialized) {
    let mine_ust_init_msg = {
      initialize_pool: {
        terraswap_lp_token: network.mine_ust_terraswap_lp_token_address,
        incentives_share: MINE_UST_ASTRO_INCENTIVES,
      },
    };

    console.log(
      `${terra.config.chainID} :: Lockdrop -::- Initialize MINE-UST LP Pool...`
    );
    let mine_ust_pool_init = await executeContract(
      terra,
      wallet,
      network.lockdrop_address,
      mine_ust_init_msg,
      [],
      "Lockdrop -::- Initialize MINE-UST LP Pool"
    );
    console.log(mine_ust_pool_init.txhash);
    console.log(
      `Lockdrop :: MINE-UST Pool successfully initialized with Lockdrop \n`
    );
    network.mine_ust_lockdrop_pool_initialized = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Initialize PSI-UST Pool in Lockdrop
  if (!network.psi_ust_lockdrop_pool_initialized) {
    let psi_ust_init_msg = {
      initialize_pool: {
        terraswap_lp_token: network.psi_ust_terraswap_lp_token_address,
        incentives_share: PSI_UST_ASTRO_INCENTIVES,
      },
    };

    console.log(
      `${terra.config.chainID} :: Lockdrop -::- Initialize PSI-UST LP Pool...`
    );
    let psi_ust_pool_init = await executeContract(
      terra,
      wallet,
      network.lockdrop_address,
      psi_ust_init_msg,
      [],
      " Lockdrop -::- Initialize PSI-UST LP Pool"
    );
    console.log(psi_ust_pool_init.txhash);
    console.log(
      `Lockdrop :: PSI-UST Pool successfully initialized with Lockdrop \n`
    );
    network.psi_ust_lockdrop_pool_initialized = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Initialize APOLLO-UST Pool with incentive
  if (!network.apollo_ust_lockdrop_pool_initialized) {
    let apollo_ust_init_msg = {
      initialize_pool: {
        terraswap_lp_token: network.apollo_ust_terraswap_lp_token_address,
        incentives_share: APOLLO_UST_ASTRO_INCENTIVES,
      },
    };
    console.log(
      `${terra.config.chainID} :: Lockdrop -::- Initialize APOLLO-UST LP Pool...`
    );
    let apollo_ust_pool_init = await executeContract(
      terra,
      wallet,
      network.lockdrop_address,
      apollo_ust_init_msg,
      [],
      "Lockdrop -::- Initialize APOLLO-UST LP Pool"
    );
    console.log(apollo_ust_pool_init.txhash);
    console.log(
      `Lockdrop :: APOLLO-UST Pool successfully initialized with Lockdrop \n`
    );
    network.apollo_ust_lockdrop_pool_initialized = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Lockdrop ::: UpdateConfig :: Update Owner to ASTRO MultiSig
  if (!network.lockdrop_multisig_made_owner) {
    console.log(
      `${terra.config.chainID} :: Lockdrop -::- UpdateConfig :: Update Owner to ASTRO MultiSig`
    );
    let tx = await executeContract(
      terra,
      wallet,
      network.lockdrop_address,
      {
        update_config: {
          new_config: {
            owner: ASTRO_MULTISIG,
            astro_token_address: undefined,
            auction_contract_address: undefined,
            generator_address: undefined,
          },
        },
      },
      [],
      "Lockdrop -::- Update Owner"
    );
    console.log(
      `Lockdrop :: Owner updated successfully to ASTRO MultiSig ---> ${tx.txhash}\n`
    );
    network.lockdrop_multisig_made_owner = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Lockdrop ::: Transfer Ownership to multiSig
  if (!network.lockdrop_ownership_transferred_to_multisig) {
    let tx = await transfer_ownership_to_multisig(
      terra,
      wallet,
      ASTRO_MULTISIG,
      network.lockdrop_address
    );
    console.log(
      `Transferred ownership of LOCKDROP contract, \n Tx hash --> ${tx.txhash} \n`
    );
    network.lockdrop_ownership_transferred_to_multisig = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Airdrop ::: Transfer Ownership to multiSig
  if (!network.airdrop_ownership_transferred_to_multisig) {
    let tx = await transfer_ownership_to_multisig(
      terra,
      wallet,
      ASTRO_MULTISIG,
      network.airdrop_address
    );
    console.log(
      `Transferred ownership of AIRDROP contract, \n Tx hash --> ${tx.txhash} \n`
    );
    network.airdrop_ownership_transferred_to_multisig = true;
    writeArtifact(network, terra.config.chainID);
  }

  // Auction ::: Transfer Ownership to multiSig
  if (!network.auction_ownership_transferred_to_multisig) {
    let tx = await transfer_ownership_to_multisig(
      terra,
      wallet,
      ASTRO_MULTISIG,
      network.auction_address
    );
    console.log(
      `Transferred ownership of AUCTION contract, \n Tx hash --> ${tx.txhash} \n`
    );
    network.auction_ownership_transferred_to_multisig = true;
    writeArtifact(network, terra.config.chainID);
  }

  writeArtifact(network, terra.config.chainID);
  console.log("FINISH");
}

main().catch(console.log);
