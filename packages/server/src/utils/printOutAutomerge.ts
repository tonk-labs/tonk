import * as Automerge from '@automerge/automerge';
import {logger} from '../logger.js';

/**
 * Prints the first `numEntries` entries of an Automerge document.
 *
 * @param doc The Automerge document to print from.
 * @param numEntries The number of entries to print.  Defaults to 5.
 * @param keyPrefix A prefix to add to the key for formatting the output. Defaults to "Entry: ".
 */
export const printFirstEntries = (
  doc: Automerge.Doc<any>,
  numEntries: number = 5,
  keyPrefix: string = 'Entry: ',
): void => {
  let count = 0;

  for (const key of Object.keys(doc)) {
    logger.info(`${keyPrefix}${key}:`, doc[key]);
    count++;

    if (count >= numEntries) {
      break;
    }
  }

  if (count === 0) {
    logger.info('Document is empty.');
  } else {
    logger.info(`Printed the first ${Math.min(numEntries, count)} entries.`);
  }
};
