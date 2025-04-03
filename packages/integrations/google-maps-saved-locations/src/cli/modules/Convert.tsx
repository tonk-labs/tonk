import React, { useState, useEffect } from "react";
import { Box, Text, useInput } from "ink";
import { useNavigate, ROUTE } from "../routes";
import { InputBox } from "../components/InputBox";
import { Button } from "../components/Button";
import { AsciiText } from "../components/AsciiText";
import { useForm } from "../hooks/useForm/useForm";
import { length } from "../hooks/useForm/rules";
import { SelectionZone } from "../components/SelectionZone/SelectionZone";
import { Selection } from "../components/SelectionZone/Selection";
import { Loader } from "../components/Loader";
import { Error } from "../components/Error";
import { convertLocations } from "../../convert/convertLocations";
import { SchemaMapping, DEFAULT_SCHEMA_MAPPING } from "../../convert/schema";
import * as fs from "fs/promises";
import * as path from "path";

export const Convert: React.FC = () => {
  const navigate = useNavigate();
  const [isConverting, setIsConverting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [apiKey, setApiKey] = useState<string>("");
  const [schemaMapping, setSchemaMapping] = useState<SchemaMapping>(
    DEFAULT_SCHEMA_MAPPING,
  );
  const [schemaMappingString, setSchemaMappingString] = useState<string>(
    JSON.stringify(DEFAULT_SCHEMA_MAPPING, null, 2),
  );

  // Load API key from .env file if available
  useEffect(() => {
    const loadApiKey = async () => {
      try {
        const envPath = path.resolve(process.cwd(), ".env");
        const envContent = await fs.readFile(envPath, "utf-8");
        const match = envContent.match(/GOOGLE_MAPS_API_KEY=([^\r\n]+)/);
        if (match && match[1]) {
          setApiKey(match[1]);
        }
      } catch (err) {
        // .env file might not exist, that's okay
      }
    };

    loadApiKey();
  }, []);

  const {
    data: values,
    errors,
    register,
    isValid,
  } = useForm({
    initialValues: {
      csvPath: "locations.csv",
      outputPath: "locations.bin",
      apiKey: "",
      userId: "7fn52mcm1f5",
      userName: "Jack",
    },
    rules: {
      csvPath: length(1),
      outputPath: length(1),
      apiKey: length(1),
      userId: length(1),
      userName: length(1),
    },
  });

  // Create input props for each field
  const inputProps = {
    csvPath: register("csvPath"),
    outputPath: register("outputPath"),
    apiKey: register("apiKey"),
    userId: register("userId"),
    userName: register("userName"),
  };

  // Update form API key when loaded from .env
  useEffect(() => {
    if (apiKey && !values.apiKey) {
      inputProps.apiKey.onChange(apiKey);
    }
  }, [apiKey, values.apiKey]);

  // Update schema mapping when the string changes
  // This function is for future implementation
  const handleSchemaMappingChange = (value: string) => {
    setSchemaMappingString(value);
    try {
      const parsed = JSON.parse(value);
      setSchemaMapping(parsed);
      setError(null);
    } catch (err) {
      setError("Invalid schema mapping JSON");
    }
  };

  const handleConvert = async () => {
    try {
      // Check if all required values are available
      if (
        !values.apiKey ||
        !values.csvPath ||
        !values.outputPath ||
        !values.userId ||
        !values.userName
      ) {
        setError("All fields are required");
        return;
      }

      setIsConverting(true);
      setError(null);
      setSuccess(null);

      // Use a try-catch block to prevent any errors from propagating up
      try {
        await convertLocations({
          apiKey: values.apiKey,
          inputCsvPath: values.csvPath,
          outputAutomergePath: values.outputPath,
          schemaMapping,
          defaultUserId: values.userId,
          defaultUserName: values.userName,
        });

        setSuccess("Conversion completed successfully!");
      } catch (conversionError) {
        console.error("Conversion error:", conversionError);
        setError(
          `Conversion failed: ${conversionError instanceof Error ? conversionError.message : String(conversionError)}`,
        );
      }
    } catch (err) {
      console.error("Unexpected error:", err);
      setError(
        `Conversion failed: ${err instanceof Error ? err.message : String(err)}`,
      );
    } finally {
      setIsConverting(false);
    }
  };

  // Track which input field is currently focused
  const [focusedField, setFocusedField] = useState(0);
  const fieldCount = 5; // Total number of input fields

  // Handle tab key to move between fields
  useInput((_input, key) => {
    if (key.tab) {
      setFocusedField((prev) => (prev + 1) % (fieldCount + 1)); // +1 for buttons
    } else if (key.shift && key.tab) {
      setFocusedField(
        (prev) => (prev - 1 + (fieldCount + 1)) % (fieldCount + 1),
      );
    }
  });

  return (
    <Box flexDirection="column" padding={1}>
      <AsciiText text="Convert Locations" font="Standard" align="center" />
      <Box marginY={1}>
        <Text>
          Convert CSV locations to Google Maps data and merge into an Automerge
          document
        </Text>
      </Box>

      <Box flexDirection="column" marginY={1}>
        <InputBox
          label="CSV Path"
          error={errors.csvPath}
          focus={focusedField === 0}
          {...inputProps.csvPath}
        />

        <InputBox
          label="Output Automerge Path"
          error={errors.outputPath}
          focus={focusedField === 1}
          {...inputProps.outputPath}
        />

        <InputBox
          label="Google Maps API Key"
          error={errors.apiKey}
          focus={focusedField === 2}
          {...inputProps.apiKey}
        />

        <InputBox
          label="Default User ID"
          error={errors.userId}
          focus={focusedField === 3}
          {...inputProps.userId}
        />

        <InputBox
          label="Default User Name"
          error={errors.userName}
          focus={focusedField === 4}
          {...inputProps.userName}
        />

        <Box flexDirection="column" marginY={1}>
          <Text>Schema Mapping (JSON):</Text>
          <Box
            borderStyle="round"
            borderColor="gray"
            padding={1}
            marginTop={1}
            flexDirection="column"
          >
            <Text wrap="wrap">{schemaMappingString}</Text>
          </Box>
          <Text>Press Tab to edit schema mapping</Text>
        </Box>

        {error && <Error text={error} />}
        {success && (
          <Box marginY={1} padding={1} borderStyle="round" borderColor="green">
            <Text color="green">{success}</Text>
          </Box>
        )}

        <SelectionZone
          prevKey="up"
          nextKey="down"
          isActive={focusedField === 5}
        >
          <Box marginY={2} justifyContent="space-between">
            <Selection>
              {(isFocused) => (
                <Button
                  onPress={handleConvert}
                  isDisabled={!isValid || isConverting || !!error}
                  isLoading={isConverting}
                  isFocused={isFocused}
                >
                  <Text bold={isFocused}>Convert</Text>
                </Button>
              )}
            </Selection>
            <Selection>
              {(isFocused) => (
                <Button
                  onPress={() => navigate(ROUTE.HOME)}
                  isFocused={isFocused}
                >
                  <Text bold={isFocused}>Back to Home</Text>
                </Button>
              )}
            </Selection>
          </Box>
        </SelectionZone>
      </Box>

      {isConverting && (
        <Loader loading type="dots">
          <Text>Converting locations...</Text>
        </Loader>
      )}
    </Box>
  );
};
